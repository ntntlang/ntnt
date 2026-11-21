//! What one SQLite session's catalog inspection reuses from its earlier ones.
//!
//! Inspection reads each object's metadata and compiles each table's and
//! view's programs under the native authorizer. Replay inspects after every
//! migration, so redoing that for every object made a history of n migrations
//! cost O(n²) metadata reads and program compiles, and building and
//! fingerprinting the whole catalog again cost O(n²) more (#421).
//!
//! The session keeps the catalog its last inspection reported. Objects are
//! grouped by the table or view they belong to: a table or view with the
//! indexes and triggers on it. An inspection still reads every
//! `sqlite_schema` row, but rebuilds only the groups whose rows, or the
//! `pragma_table_list` facts their objects use, changed. What a group
//! contributes is a function of exactly those.
//!
//! A program's coverage depends on other objects too. While compiling it,
//! SQLite resolves the objects named in the stored SQL of what it compiles
//! (trigger bodies, view queries, foreign-key parents), and for each table the
//! program writes, the tables whose foreign keys reference it. `dependencies`
//! over-approximates that set from the stored SQL texts and the foreign-key
//! lists. A group's programs are not compiled again while they compiled under
//! the same foreign-key enforcement and every group they depend on is
//! unchanged; programs that failed are always compiled again.
use super::model::Catalog;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The key of a name's group: SQLite compares names ASCII case-insensitively.
pub(super) fn group_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Every name `sql` can refer to, ASCII-lowercased: each run of identifier
/// characters anywhere in it, quoted text and comments included, and the
/// unescaped contents of each quoted token. This is a superset of the
/// identifiers SQLite's tokenizer finds in the text.
pub(super) fn names(sql: &str, out: &mut HashSet<String>) {
    let bytes = sql.as_bytes();
    // SQLite's identifier characters, including `$` and every byte of a
    // multi-byte character. Runs never split a UTF-8 sequence.
    let identifier = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80;
    let mut i = 0;
    while i < bytes.len() {
        if identifier(bytes[i]) {
            let start = i;
            while i < bytes.len() && identifier(bytes[i]) {
                i += 1;
            }
            out.insert(sql[start..i].to_ascii_lowercase());
        } else {
            i += 1;
        }
    }
    // Quoted identifiers and strings, skipping comments as SQLite does.
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            quote @ (b'\'' | b'"' | b'`') => {
                let mut text = String::new();
                let mut start = i + 1;
                i += 1;
                loop {
                    match bytes.get(i) {
                        // An unterminated token runs to the end of the text.
                        None => {
                            text.push_str(&sql[start..]);
                            break;
                        }
                        Some(&b) if b == quote => {
                            text.push_str(&sql[start..i]);
                            if bytes.get(i + 1) == Some(&quote) {
                                // A doubled quote stands for one quote.
                                text.push(quote as char);
                                i += 2;
                                start = i;
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        Some(_) => i += 1,
                    }
                }
                out.insert(text.to_ascii_lowercase());
            }
            b'[' => {
                let start = i + 1;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                out.insert(sql[start.min(bytes.len())..i].to_ascii_lowercase());
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// Metadata of one table, processed as the catalog reports it.
pub(super) struct Table {
    /// Catalog key and entry of each column, defaults redacted.
    pub columns: Vec<(String, Value)>,
    pub column_names: Vec<String>,
    /// Whether every column is visible (`hidden` = 0).
    pub visible: bool,
    pub foreign_keys: Value,
    /// Whether no foreign key references a reserved or internal table.
    pub foreign_keys_supported: bool,
}
/// Metadata of one index, processed as the catalog reports it.
pub(super) struct Index {
    pub columns: Value,
    pub columns_supported: bool,
    pub properties: Value,
    pub properties_supported: bool,
}

/// What a group contributes to the catalog, valid while its rows and the
/// `pragma_table_list` facts its objects use are unchanged.
pub(super) struct Output {
    /// For each of the group's rows: the `pragma_table_list` row of a table,
    /// whether a trigger's table is an ordinary table, or null.
    pub facts: Vec<Value>,
    /// For each of the group's rows, the catalog objects it adds (its
    /// columns and itself); counted against the 10,000-object bound.
    pub contributions: Vec<usize>,
    pub entries: Vec<(String, Value)>,
    /// Keys outside metadata coverage, by type and name.
    pub outside: Vec<((String, String), String)>,
    /// Catalog key and SQL of each program to compile.
    pub programs: Vec<(String, String)>,
    /// The tables its tables' foreign keys reference, as written.
    pub parents: Vec<String>,
}
impl Output {
    pub fn new(facts: Vec<Value>) -> Self {
        Self {
            contributions: vec![0; facts.len()],
            facts,
            entries: Vec::new(),
            outside: Vec::new(),
            programs: Vec::new(),
            parents: Vec::new(),
        }
    }
}

struct Group {
    /// The group's `sqlite_schema` rows.
    rows: Vec<Vec<Value>>,
    hash: [u8; 32],
    /// Names in the stored SQL of every row of the group.
    names: HashSet<String>,
    /// Names in the stored SQL of the group's triggers.
    trigger_names: HashSet<String>,
    output: Option<Output>,
    /// Foreign-key enforcement and dependency signature under which every
    /// program of `output` compiled.
    compiled: Option<(bool, [u8; 32])>,
}

/// Work counters, for tests that prove inspection cost per migration is bounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Stats {
    /// Catalog inspections.
    pub catalogs: u64,
    /// Per-object metadata queries (table, foreign-key and index pragmas).
    pub metadata_queries: u64,
    /// Programs compiled under the authorizer.
    pub programs_compiled: u64,
    /// Allowlists of resolvable relations read to compile them.
    pub allowlists: u64,
}

/// The catalog of one session as of its last inspection, and what produced it.
#[derive(Default)]
pub(super) struct Cache {
    groups: HashMap<String, Group>,
    /// `schema:main` and the entries of every group's output.
    objects: BTreeMap<String, Value>,
    /// Each object's serialized form, for the catalog's fingerprint.
    fragments: BTreeMap<String, Vec<u8>>,
    /// Keys outside metadata coverage, in `sqlite_schema` order.
    outside: BTreeMap<(String, String), String>,
    /// What the last inspection found outside coverage.
    uncovered: Vec<String>,
    pub stats: Stats,
}

impl Cache {
    /// Start an inspection of these `sqlite_schema` rows (type, name,
    /// tbl_name, sql, bounds): keep each group whose rows are unchanged and
    /// forget the others. Returns each row's group and position in it.
    pub fn refresh(&mut self, rows: &[Vec<Value>]) -> Vec<(String, usize)> {
        if self.objects.is_empty() {
            self.insert(
                "schema:main".into(),
                json!({"engine":"sqlite","scope":"entire_main_file"}),
            );
        }
        let mut contents: HashMap<String, Vec<&Vec<Value>>> = HashMap::new();
        let mut positions = Vec::with_capacity(rows.len());
        for row in rows {
            let key = group_key(row[2].as_str().unwrap_or_default());
            let rows = contents.entry(key.clone()).or_default();
            positions.push((key, rows.len()));
            rows.push(row);
        }
        let mut groups = HashMap::with_capacity(contents.len());
        for (key, rows) in contents {
            let group = match self.groups.remove(&key) {
                Some(group)
                    if group.rows.len() == rows.len()
                        && group.rows.iter().zip(&rows).all(|(a, b)| a == *b) =>
                {
                    group
                }
                old => {
                    if let Some(output) = old.and_then(|group| group.output) {
                        self.retire(&output);
                    }
                    let mut hash = Sha256::new();
                    for row in &rows {
                        let bytes = serde_json::to_vec(row).expect("catalog rows serialize");
                        hash.update((bytes.len() as u64).to_be_bytes());
                        hash.update(bytes);
                    }
                    let (mut names_, mut trigger_names) = (HashSet::new(), HashSet::new());
                    for row in &rows {
                        if let Some(sql) = row[3].as_str() {
                            names(sql, &mut names_);
                            if row[0] == "trigger" {
                                names(sql, &mut trigger_names);
                            }
                        }
                    }
                    Group {
                        rows: rows.iter().map(|row| (*row).clone()).collect(),
                        hash: hash.finalize().into(),
                        names: names_,
                        trigger_names,
                        output: None,
                        compiled: None,
                    }
                }
            };
            groups.insert(key, group);
        }
        // Groups whose objects are gone.
        let removed: Vec<Output> = self
            .groups
            .drain()
            .filter_map(|(_, group)| group.output)
            .collect();
        for output in &removed {
            self.retire(output);
        }
        self.groups = groups;
        positions
    }
    /// Forget the output of these groups: the `pragma_table_list` facts
    /// their objects use changed.
    pub fn forget(&mut self, groups: &HashSet<&str>) {
        let mut stale = Vec::new();
        for group in groups {
            if let Some(group) = self.groups.get_mut(*group) {
                group.compiled = None;
                stale.extend(group.output.take());
            }
        }
        for output in &stale {
            self.retire(output);
        }
    }
    pub fn output(&self, group: &str) -> Option<&Output> {
        self.groups.get(group)?.output.as_ref()
    }
    /// Make `output` the group's contribution to the catalog.
    pub fn install(&mut self, group: &str, output: Output) {
        for (key, value) in &output.entries {
            self.insert(key.clone(), value.clone());
        }
        for (order, key) in &output.outside {
            self.outside.insert(order.clone(), key.clone());
        }
        if let Some(group) = self.groups.get_mut(group) {
            group.output = Some(output);
            group.compiled = None;
        }
    }
    fn insert(&mut self, key: String, value: Value) {
        let mut bytes = serde_json::to_vec(&key).expect("catalog keys serialize");
        bytes.push(b':');
        serde_json::to_writer(&mut bytes, &value).expect("catalog values serialize");
        self.fragments.insert(key.clone(), bytes);
        self.objects.insert(key, value);
    }
    fn retire(&mut self, output: &Output) {
        for (key, _) in &output.entries {
            self.objects.remove(key);
            self.fragments.remove(key);
        }
        for (order, _) in &output.outside {
            self.outside.remove(order);
        }
    }
    /// Whether every object is within metadata coverage.
    pub fn within_metadata_coverage(&self) -> bool {
        self.outside.is_empty()
    }
    /// The programs of each group not compiled under this foreign-key
    /// enforcement and signature, with that signature.
    pub fn pending(
        &self,
        signatures: &HashMap<String, [u8; 32]>,
        enforced: Option<bool>,
    ) -> Vec<(String, [u8; 32], Vec<(String, String)>)> {
        let mut pending = Vec::new();
        for (key, group) in &self.groups {
            let (Some(output), Some(&signature)) = (&group.output, signatures.get(key)) else {
                continue;
            };
            let current = enforced.map(|enforced| (enforced, signature));
            if output.programs.is_empty() || (current.is_some() && group.compiled == current) {
                continue;
            }
            pending.push((key.clone(), signature, output.programs.clone()));
        }
        pending
    }
    /// Record how a group's programs last compiled: the foreign-key
    /// enforcement and signature if every one compiled, else `None`, so that
    /// a failure is compiled again at the next inspection.
    pub fn mark_compiled(&mut self, group: &str, compiled: Option<(bool, [u8; 32])>) {
        if let Some(group) = self.groups.get_mut(group) {
            group.compiled = compiled;
        }
    }
    /// Record what this inspection found outside coverage: the objects
    /// outside metadata coverage in `sqlite_schema` order, or else the
    /// sorted keys of the programs that did not compile.
    pub fn finish(&mut self, failures: Option<Vec<String>>) {
        self.uncovered = match failures {
            Some(mut failures) => {
                failures.sort();
                failures.dedup();
                failures
            }
            None => self.outside.values().cloned().collect(),
        };
    }
    pub fn uncovered(&self) -> &[String] {
        &self.uncovered
    }
    pub fn catalog(&self) -> Catalog {
        Catalog {
            objects: self.objects.clone(),
            uncovered: self.uncovered.clone(),
        }
    }
    /// `json_digest` of `catalog()`, from each object's stored form.
    pub fn fingerprint(&self) -> String {
        super::inventory::catalog_digest(
            self.fragments.values().map(Vec::as_slice),
            &self.uncovered,
        )
    }
    /// The dependency signature of each group, given each object's name and
    /// group key. Call once every group has its output.
    pub fn signatures(&self, objects: &[(String, String)]) -> HashMap<String, [u8; 32]> {
        // A name resolves to every group with an object of that name, and to
        // the group it is the key of. Names are not unique across kinds: a
        // trigger on one table may share its name with another table.
        let mut resolve: HashMap<&str, Vec<&str>> = HashMap::new();
        for (name, group) in objects {
            if let Some((key, _)) = self.groups.get_key_value(group) {
                resolve.entry(name.as_str()).or_default().push(key.as_str());
            }
        }
        for key in self.groups.keys() {
            resolve.entry(key.as_str()).or_default().push(key.as_str());
        }
        let resolved = |names: &HashSet<String>| -> Vec<&str> {
            let mut groups: Vec<&str> = names
                .iter()
                .filter_map(|name| resolve.get(name.as_str()))
                .flatten()
                .copied()
                .collect();
            groups.sort_unstable();
            groups.dedup();
            groups
        };
        let mut graph: HashMap<&str, Edges> = self
            .groups
            .iter()
            .map(|(key, group)| {
                let edges = Edges {
                    names: resolved(&group.names),
                    triggers: resolved(&group.trigger_names),
                    referencing: Vec::new(),
                };
                (key.as_str(), edges)
            })
            .collect();
        // Tables whose foreign keys reference each table.
        for (key, group) in &self.groups {
            for parent in group.output.iter().flat_map(|output| &output.parents) {
                for parent in resolve
                    .get(group_key(parent).as_str())
                    .into_iter()
                    .flatten()
                {
                    if let Some(edges) = graph.get_mut(parent) {
                        edges.referencing.push(key.as_str());
                    }
                }
            }
        }
        self.groups
            .iter()
            .map(|(root, _)| {
                let mut hash = Sha256::new();
                for key in dependencies(root, &graph) {
                    hash.update((key.len() as u64).to_be_bytes());
                    hash.update(key.as_bytes());
                    hash.update(self.groups[key].hash);
                }
                (root.clone(), hash.finalize().into())
            })
            .collect()
    }
}

/// Names resolved to groups, for one group.
struct Edges<'a> {
    /// Groups named in the stored SQL of any of its rows.
    names: Vec<&'a str>,
    /// Groups named in the stored SQL of its triggers.
    triggers: Vec<&'a str>,
    /// Tables whose foreign keys reference it.
    referencing: Vec<&'a str>,
}

/// The groups a program rooted at `root` may read while it compiles.
fn dependencies<'a>(root: &'a str, graph: &HashMap<&'a str, Edges<'a>>) -> BTreeSet<&'a str> {
    // Tables the program may write: the root, whatever the triggers of a
    // written table name, and the tables whose foreign keys reference a
    // written table (SQLite reads them, and their actions write them).
    let mut written = BTreeSet::from([root]);
    let mut queue = vec![root];
    while let Some(key) = queue.pop() {
        let edges = &graph[key];
        for &next in edges.triggers.iter().chain(&edges.referencing) {
            if written.insert(next) {
                queue.push(next);
            }
        }
    }
    // Everything named in the stored SQL of those, transitively.
    let mut read = written.clone();
    let mut queue: Vec<&str> = written.into_iter().collect();
    while let Some(key) = queue.pop() {
        for &next in &graph[key].names {
            if read.insert(next) {
                queue.push(next);
            }
        }
    }
    read
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(sql: &str) -> HashSet<String> {
        let mut out = HashSet::new();
        names(sql, &mut out);
        out
    }

    #[test]
    fn names_include_every_spelling_sqlite_resolves() {
        let found = all(
            r#"CREATE TRIGGER x AFTER INSERT ON "My ""T""" BEGIN INSERT INTO [a b] SELECT * FROM `c``d` JOIN main.Audit$1 JOIN "café"; END"#,
        );
        for name in [
            "x", "my \"t\"", "a b", "c`d", "main", "audit$1", "café", "insert",
        ] {
            assert!(found.contains(name), "{name}: {found:?}");
        }
        // A quote inside a comment does not hide what follows it.
        let found = all("CREATE VIEW v AS SELECT 1 -- it's\n FROM \"t\"\"x\" /* \" */ JOIN u");
        for name in ["t\"x", "u", "v"] {
            assert!(found.contains(name), "{name}: {found:?}");
        }
        // Unterminated quoted text still yields its contents.
        assert!(all("SELECT \"abc").contains("abc"));
        assert!(all("SELECT [abc").contains("abc"));
        // String literals count too (table-valued functions take names).
        assert!(all("SELECT * FROM pragma_table_info('Accounts')").contains("accounts"));
    }
}
