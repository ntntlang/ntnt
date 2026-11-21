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
//! A program's coverage depends on other objects too. SQLite compiles a
//! program as a tree: the top-level statement, and one sub-program for each
//! trigger and each foreign-key action it fires, each sub-program coded once
//! per compile. A program compiles if and only if no part of that tree fails
//! and no sub-program would be nested `SQLITE_LIMIT_TRIGGER_DEPTH` levels
//! deep. Each part fails or not on its own: what it reads is the objects
//! named in its stored SQL, the foreign-key parents and children of the
//! tables it writes, and the authorizer's allowlist, never the part that
//! called it.
//!
//! A group's programs are compiled again unless their last compile succeeded
//! under the same foreign-key enforcement and one of these holds (`decide`):
//!
//! - Nothing they read changed: the `signature` of what the tree reads
//!   outside the subtrees its certifiers vouch for is the same, and every
//!   certifier's own tree is unchanged.
//! - The signature is the same, every certifier compiled now, and the tree
//!   cannot reach the nesting limit (`depth`).
//!
//! A certifier is a table C that references a written table P, where every
//! part of the tree that involves C's key is also part of what C's own
//! programs compile, so C compiling vouches for it:
//!
//! - In P's own programs, for each such key: finding the unique index of
//!   P's key (as C's INSERT does to look up its parent), scanning C by its
//!   key columns (reading C, which C's INSERT may write; the comparison uses
//!   the collation C's parent lookup loads), and the key's action. An ON
//!   DELETE CASCADE action is `DELETE FROM C WHERE ...`, which codes exactly
//!   what C's DELETE codes, plus a WHERE clause on two existing tables. A
//!   RESTRICT action is a read of C. A SET NULL action, or ON UPDATE CASCADE,
//!   is an UPDATE of C's key columns under OR ABORT; when C has no triggers
//!   and no key into C whose parent columns that UPDATE changes has an ON
//!   UPDATE action, it codes no sub-programs and checks a subset of what C's
//!   UPDATE of every column checks. Such keys are left out of P's
//!   signature. The first one also changes which code P's programs take (a
//!   DELETE of a table with no foreign keys clears it, an UPDATE that
//!   changes no key skips foreign-key processing, a REPLACE conflict may
//!   delete rows), but what that code adds is again those parts of each
//!   key, foreign-key work on P's own keys that P's INSERT already does, and
//!   cursors on P's indexes that P's INSERT already opens.
//! - Deeper in the tree, only ON DELETE CASCADE keys certify the subtree
//!   below them; the child's own rows stay in the signature.
//!
//! SET DEFAULT and the other actions are never certified: their subtrees
//! are part of the signature.
//!
//! The signature covers, transitively, each written table's group, the
//! groups named in its stored SQL, the groups its triggers name (as
//! written), and its foreign-key children as above. A table that is only
//! read contributes its own group: reading a table resolves its columns and
//! indexes, never what its own stored SQL names. A view that is read
//! contributes what it names, transitively.
//!
//! Programs that failed are always compiled again. Groups that certify each
//! other in a cycle are compiled again together when anything in the cycle
//! changed.
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

/// What a foreign key does to the referencing rows when a parent row is
/// deleted or its key updated, as `pragma_foreign_key_list` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Action {
    NoAction,
    Restrict,
    SetNull,
    SetDefault,
    Cascade,
    /// Anything else, treated as an action that may do anything.
    Unknown,
}
impl Action {
    pub fn parse(text: Option<&str>) -> Self {
        match text {
            Some("NO ACTION") => Self::NoAction,
            Some("RESTRICT") => Self::Restrict,
            Some("SET NULL") => Self::SetNull,
            Some("SET DEFAULT") => Self::SetDefault,
            Some("CASCADE") => Self::Cascade,
            _ => Self::Unknown,
        }
    }
}

/// One foreign key of a table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ForeignKey {
    /// The parent table as written.
    pub parent: String,
    /// The child columns, ASCII-lowercased.
    pub from: Vec<String>,
    /// The parent key columns, ASCII-lowercased; `None` where the key is the
    /// parent's primary key.
    pub to: Vec<Option<String>>,
    pub on_update: Action,
    pub on_delete: Action,
}
impl ForeignKey {
    /// The keys of `pragma_foreign_key_list` rows (id, seq, table, from, to,
    /// on_update, on_delete, match), ordered by id and seq.
    pub fn parse(rows: &[Value]) -> Vec<Self> {
        let lower = |value: &Value| value.as_str().map(str::to_ascii_lowercase);
        let mut keys: Vec<(Value, Self)> = Vec::new();
        for row in rows {
            match keys.last_mut() {
                Some((id, key)) if *id == row[0] => {
                    key.from.push(lower(&row[3]).unwrap_or_default());
                    key.to.push(lower(&row[4]));
                }
                _ => keys.push((
                    row[0].clone(),
                    Self {
                        parent: row[2].as_str().unwrap_or_default().to_owned(),
                        from: vec![lower(&row[3]).unwrap_or_default()],
                        to: vec![lower(&row[4])],
                        on_update: Action::parse(row[5].as_str()),
                        on_delete: Action::parse(row[6].as_str()),
                    },
                )),
            }
        }
        keys.into_iter().map(|(_, key)| key).collect()
    }
    /// Whether an UPDATE of the parent that changes `columns` changes this
    /// key, as SQLite's `fkParentIsModified` decides: a named key column is
    /// changed, or the key is the primary key and a primary key column is.
    fn changed_by(&self, columns: &[String], primary_key: &[String]) -> bool {
        self.to.iter().any(|to| match to {
            Some(column) => columns.contains(column),
            None => columns.iter().any(|column| primary_key.contains(column)),
        })
    }
}

/// Metadata of one table, processed as the catalog reports it.
pub(super) struct Table {
    /// Catalog key and entry of each column, defaults redacted.
    pub columns: Vec<(String, Value)>,
    pub column_names: Vec<String>,
    /// The primary key columns, ASCII-lowercased.
    pub primary_key: Vec<String>,
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
    /// The foreign keys of its table.
    pub foreign_keys: Vec<ForeignKey>,
    /// Its table's primary key columns, ASCII-lowercased.
    pub primary_key: Vec<String>,
}
impl Output {
    pub fn new(facts: Vec<Value>) -> Self {
        Self {
            contributions: vec![0; facts.len()],
            facts,
            entries: Vec::new(),
            outside: Vec::new(),
            programs: Vec::new(),
            foreign_keys: Vec::new(),
            primary_key: Vec::new(),
        }
    }
}

/// How a group's programs last compiled, each one of them successfully.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Compiled {
    /// Foreign-key enforcement.
    enforced: bool,
    signature: [u8; 32],
}

/// What the last plan found for a group's programs, and every group it read
/// to find it.
struct Planned {
    signature: [u8; 32],
    /// Groups whose programs must compile now for these to be reused after a
    /// change below them, sorted.
    certifiers: Vec<String>,
    reads: Vec<String>,
}

struct Group {
    /// The group's `sqlite_schema` rows.
    rows: Vec<Vec<Value>>,
    hash: [u8; 32],
    /// Names in the stored SQL of every row of the group.
    names: HashSet<String>,
    /// Names in the stored SQL of the group's triggers.
    trigger_names: HashSet<String>,
    /// The names of its objects, ASCII-lowercased: they resolve to it.
    object_names: Vec<String>,
    /// How many of its rows are triggers.
    triggers: usize,
    /// Whether it is a view's group.
    view: bool,
    output: Option<Output>,
    compiled: Option<Compiled>,
    /// The groups its names and its triggers' names resolve to, until one of
    /// those names may resolve differently.
    resolved: Option<(Vec<String>, Vec<String>)>,
    /// The foreign keys into it as of the last plan: the child and the key's
    /// position in the child's output.
    into: Vec<(String, usize)>,
    planned: Option<Planned>,
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
    /// Each name, to the groups it resolves to: every group with an object
    /// of that name, and the group it is the key of. Names are not unique
    /// across kinds: a trigger on one table may share another table's name.
    resolve: HashMap<String, BTreeSet<String>>,
    /// Each name, to the groups whose stored SQL has it.
    mentions: HashMap<String, HashSet<String>>,
    /// Each group, to the groups whose last plan read it.
    readers: HashMap<String, HashSet<String>>,
    /// Groups whose rows, facts, resolved names or keys into them changed
    /// since the last plan.
    dirty: HashSet<String>,
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
        let mut renamed = HashSet::new();
        for (key, rows) in contents {
            let group = match self.groups.remove(&key) {
                Some(group)
                    if group.rows.len() == rows.len()
                        && group.rows.iter().zip(&rows).all(|(a, b)| a == *b) =>
                {
                    group
                }
                old => {
                    if let Some(old) = old {
                        self.unlink(&key, old, &mut renamed);
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
                    let group = Group {
                        rows: rows.iter().map(|row| (*row).clone()).collect(),
                        hash: hash.finalize().into(),
                        names: names_,
                        trigger_names,
                        object_names: rows
                            .iter()
                            .map(|row| group_key(row[1].as_str().unwrap_or_default()))
                            .collect(),
                        triggers: rows.iter().filter(|row| row[0] == "trigger").count(),
                        view: rows.iter().any(|row| row[0] == "view"),
                        output: None,
                        compiled: None,
                        resolved: None,
                        into: Vec::new(),
                        planned: None,
                    };
                    self.link(&key, &group, &mut renamed);
                    group
                }
            };
            groups.insert(key, group);
        }
        // Groups whose objects are gone.
        for (key, group) in std::mem::take(&mut self.groups) {
            self.unlink(&key, group, &mut renamed);
        }
        self.groups = groups;
        // Whatever names a name that now resolves differently may read
        // something else.
        for name in renamed {
            for group in self.mentions.get(&name).into_iter().flatten() {
                if let Some(group) = self.groups.get_mut(group) {
                    group.resolved = None;
                }
                self.dirty.insert(group.clone());
            }
        }
        positions
    }
    /// Index a new group's names.
    fn link(&mut self, key: &str, group: &Group, renamed: &mut HashSet<String>) {
        for name in group.object_names.iter().map(String::as_str).chain([key]) {
            self.resolve
                .entry(name.to_owned())
                .or_default()
                .insert(key.to_owned());
            renamed.insert(name.to_owned());
        }
        for name in &group.names {
            self.mentions
                .entry(name.clone())
                .or_default()
                .insert(key.to_owned());
        }
        self.dirty.insert(key.to_owned());
    }
    /// Drop a replaced or removed group from the catalog and the indexes.
    fn unlink(&mut self, key: &str, group: Group, renamed: &mut HashSet<String>) {
        if let Some(output) = &group.output {
            self.retire(output);
        }
        for name in group.object_names.iter().map(String::as_str).chain([key]) {
            if let Some(groups) = self.resolve.get_mut(name) {
                groups.remove(key);
                if groups.is_empty() {
                    self.resolve.remove(name);
                }
            }
            renamed.insert(name.to_owned());
        }
        for name in &group.names {
            if let Some(groups) = self.mentions.get_mut(name) {
                groups.remove(key);
                if groups.is_empty() {
                    self.mentions.remove(name);
                }
            }
        }
        for read in group.planned.iter().flat_map(|p| &p.reads) {
            if let Some(readers) = self.readers.get_mut(read) {
                readers.remove(key);
            }
        }
        self.dirty.insert(key.to_owned());
    }
    /// Forget the output of these groups: the `pragma_table_list` facts
    /// their objects use changed.
    pub fn forget(&mut self, groups: &HashSet<&str>) {
        let mut stale = Vec::new();
        for key in groups {
            if let Some(group) = self.groups.get_mut(*key) {
                group.compiled = None;
                stale.extend(group.output.take());
                self.dirty.insert((*key).to_owned());
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
        if let Some(entry) = self.groups.get_mut(group) {
            entry.output = Some(output);
            entry.compiled = None;
            self.dirty.insert(group.to_owned());
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
    /// Bring every group's plan up to date and return the groups with
    /// programs in the order `decide` takes them: one unit per set of groups
    /// that certify each other, every certifier's unit before the units it
    /// certifies. Call once every group has its output.
    pub fn plan(&mut self) -> Vec<Vec<String>> {
        // The foreign keys into each group. SQLite finds a table's children
        // by its name, so a key into P is a key into P's group only.
        let mut into: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        for (child, group) in &self.groups {
            for (i, key) in group
                .output
                .iter()
                .flat_map(|o| &o.foreign_keys)
                .enumerate()
            {
                into.entry(group_key(&key.parent))
                    .or_default()
                    .push((child.clone(), i));
            }
        }
        let mut changed = Vec::new();
        for (key, group) in &self.groups {
            let mut now = into.remove(key).unwrap_or_default();
            now.sort_unstable();
            if now != group.into {
                changed.push((key.clone(), now));
            }
        }
        for (key, now) in changed {
            self.groups.get_mut(&key).unwrap().into = now;
            self.dirty.insert(key);
        }
        for group in self.groups.values_mut() {
            if group.resolved.is_none() {
                let resolved = |names: &HashSet<String>| {
                    let mut groups: Vec<String> = names
                        .iter()
                        .filter_map(|name| self.resolve.get(name))
                        .flatten()
                        .cloned()
                        .collect();
                    groups.sort_unstable();
                    groups.dedup();
                    groups
                };
                group.resolved = Some((resolved(&group.names), resolved(&group.trigger_names)));
            }
        }
        // Plans to redo: those of changed groups and of every group whose
        // plan read one.
        let mut stale: BTreeSet<String> = BTreeSet::new();
        for key in std::mem::take(&mut self.dirty) {
            if let Some(readers) = self.readers.get(&key) {
                stale.extend(readers.iter().cloned());
            }
            stale.insert(key);
        }
        for (key, group) in &self.groups {
            if group.planned.is_none() && Self::has_programs(group) {
                stale.insert(key.clone());
            }
        }
        // A removed group's readers are planned again now; nothing reads it
        // any more.
        let groups = &self.groups;
        self.readers.retain(|key, _| groups.contains_key(key));
        for key in stale {
            let planned = match self.groups.get(&key) {
                Some(group) if Self::has_programs(group) => Some(self.signature(&key)),
                _ => None,
            };
            let previous = match self.groups.get_mut(&key) {
                Some(group) => std::mem::replace(&mut group.planned, planned),
                None => None,
            };
            for read in previous.iter().flat_map(|p| &p.reads) {
                if let Some(readers) = self.readers.get_mut(read) {
                    readers.remove(&key);
                }
            }
            let reads = self.groups.get(&key).and_then(|g| g.planned.as_ref());
            for read in reads.iter().flat_map(|p| &p.reads) {
                self.readers
                    .entry(read.clone())
                    .or_default()
                    .insert(key.clone());
            }
        }
        let mut keys: Vec<&String> = self
            .groups
            .iter()
            .filter(|(_, g)| Self::has_programs(g))
            .map(|(k, _)| k)
            .collect();
        keys.sort_unstable();
        let certifiers = |key: &str| -> &[String] {
            self.groups[key]
                .planned
                .as_ref()
                .map_or(&[], |p| &p.certifiers)
        };
        if keys.iter().all(|k| certifiers(k).is_empty()) {
            return keys.into_iter().map(|k| vec![k.clone()]).collect();
        }
        let at: HashMap<&str, usize> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        let edges: Vec<Vec<usize>> = keys
            .iter()
            .map(|k| {
                certifiers(k)
                    .iter()
                    .filter_map(|c| at.get(c.as_str()).copied())
                    .collect()
            })
            .collect();
        components(&edges)
            .into_iter()
            .map(|unit| unit.into_iter().map(|i| keys[i].clone()).collect())
            .collect()
    }
    fn has_programs(group: &Group) -> bool {
        group
            .output
            .as_ref()
            .is_some_and(|o| !o.programs.is_empty())
    }
    /// Decide each group's programs in `plan`'s order: reuse their last
    /// verdict where the module's rules allow it, otherwise `compile` them
    /// (it returns the keys of those that did not compile). Returns the keys
    /// of every program that did not compile.
    pub fn decide(
        &mut self,
        units: &[Vec<String>],
        enforced: Option<bool>,
        depth_limit: u64,
        mut compile: impl FnMut(&[(String, String)]) -> Vec<String>,
    ) -> Vec<String> {
        // Whether each decided group compiles now, and whether its tree may
        // differ from the one its last verdict was for.
        let mut verdicts: HashMap<&str, (bool, bool)> = HashMap::new();
        let mut depths: Option<HashMap<String, u64>> = None;
        let mut failures = Vec::new();
        for unit in units {
            let planned = |key: &str| self.groups[key].planned.as_ref().unwrap();
            // The last verdict was for this signature and enforcement.
            let same = |key: &str| {
                self.groups[key].compiled.is_some_and(|c| {
                    Some(c.enforced) == enforced && c.signature == planned(key).signature
                })
            };
            // And every certifier outside the unit kept its tree too.
            let unchanged = |key: &str| {
                same(key)
                    && planned(key).certifiers.iter().all(|c| {
                        unit.contains(c)
                            || verdicts
                                .get(c.as_str())
                                .is_some_and(|&(ok, changed)| ok && !changed)
                    })
            };
            let first = unit[0].as_str();
            let cyclic = unit.len() > 1 || planned(first).certifiers.iter().any(|c| c == first);
            let (reuse, changed) = if cyclic {
                // Groups that certify each other are compiled again together
                // unless none of their trees changed.
                let unchanged = unit.iter().all(|k| unchanged(k));
                (unchanged, !unchanged)
            } else if unchanged(first) {
                (true, false)
            } else if same(first)
                && planned(first)
                    .certifiers
                    .iter()
                    .all(|c| verdicts.get(c.as_str()).is_some_and(|&(ok, _)| ok))
                && depths.get_or_insert_with(|| self.depths())[first] <= depth_limit
            {
                (true, true)
            } else {
                (false, true)
            };
            for key in unit {
                if reuse {
                    verdicts.insert(key, (true, changed));
                    continue;
                }
                let group = &self.groups[key.as_str()];
                let programs = group.output.as_ref().unwrap().programs.clone();
                let signature = group.planned.as_ref().unwrap().signature;
                self.stats.programs_compiled += programs.len() as u64;
                let failed = compile(&programs);
                let ok = failed.is_empty();
                failures.extend(failed);
                self.groups.get_mut(key.as_str()).unwrap().compiled =
                    enforced.filter(|_| ok).map(|enforced| Compiled {
                        enforced,
                        signature,
                    });
                verdicts.insert(key, (ok, true));
            }
        }
        failures
    }
    fn group(&self, key: &str) -> &Group {
        &self.groups[key]
    }
    fn resolved(&self, key: &str) -> (&[String], &[String]) {
        let (names, triggers) = self.group(key).resolved.as_ref().unwrap();
        (names, triggers)
    }
    /// The key at this position of `child`'s foreign keys.
    fn key(&self, child: &str, position: usize) -> &ForeignKey {
        &self.group(child).output.as_ref().unwrap().foreign_keys[position]
    }
    /// Whether the UPDATE of `columns` of `child` that a SET NULL or
    /// ON UPDATE CASCADE action codes fires no sub-program: the child has no
    /// triggers, and no key into it that the UPDATE changes has an ON UPDATE
    /// action. (OR ABORT rules out REPLACE.)
    fn leaf<'a>(
        &'a self,
        child: &'a str,
        columns: &[String],
        reads: &mut BTreeSet<&'a str>,
    ) -> bool {
        reads.insert(child);
        let group = self.group(child);
        let Some(output) = &group.output else {
            return false;
        };
        let mut leaf = group.triggers == 0;
        for (grandchild, position) in &group.into {
            reads.insert(grandchild);
            let key = self.key(grandchild, *position);
            if key.on_update != Action::NoAction && key.changed_by(columns, &output.primary_key) {
                leaf = false;
            }
        }
        leaf
    }
    /// Whether `key` of `child` into the group being planned is certified
    /// by the child's own programs (see the module documentation).
    fn simple<'a>(
        &'a self,
        child: &'a str,
        key: &ForeignKey,
        reads: &mut BTreeSet<&'a str>,
    ) -> bool {
        let mut leaf = || self.leaf(child, &key.from, reads);
        (match key.on_delete {
            Action::NoAction | Action::Restrict | Action::Cascade => true,
            Action::SetNull => leaf(),
            _ => false,
        }) && match key.on_update {
            Action::NoAction | Action::Restrict => true,
            Action::Cascade | Action::SetNull => leaf(),
            _ => false,
        }
    }
    /// The signature of what `root`'s programs read outside certified
    /// subtrees, the certifiers of those subtrees, and every group read.
    fn signature(&self, root: &str) -> Planned {
        let mut reads: BTreeSet<&str> = BTreeSet::new();
        let mut seen: HashSet<(Use, &str)> = HashSet::from([(Use::Written, root)]);
        let mut stack = vec![(Use::Written, root)];
        let mut certifiers = BTreeSet::new();
        while let Some((use_, group)) = stack.pop() {
            reads.insert(group);
            let mut visit = |node| {
                if seen.insert(node) {
                    stack.push(node);
                }
            };
            let (names, triggers) = self.resolved(group);
            if use_ == Use::Read {
                // Reading a view reads what it names; reading a table, only
                // the table and its indexes.
                if self.group(group).view {
                    for named in names {
                        visit((Use::Read, named));
                    }
                }
                continue;
            }
            visit((Use::Read, group));
            for named in names {
                visit((Use::Read, named));
            }
            for named in triggers {
                visit((Use::Written, named));
            }
            for (child, position) in &self.group(group).into {
                let child = child.as_str();
                reads.insert(child);
                let key = self.key(child, *position);
                if group == root && self.simple(child, key, &mut reads) {
                    certifiers.insert(child);
                    continue;
                }
                let updates = |action| {
                    matches!(
                        action,
                        Action::SetNull | Action::SetDefault | Action::Cascade | Action::Unknown
                    )
                };
                if matches!(
                    key.on_delete,
                    Action::SetNull | Action::SetDefault | Action::Unknown
                ) || updates(key.on_update)
                {
                    visit((Use::Written, child));
                } else {
                    visit((Use::Read, child));
                    if key.on_delete == Action::Cascade {
                        certifiers.insert(child);
                    }
                }
            }
        }
        let mut nodes: Vec<(Use, &str)> = seen.into_iter().collect();
        nodes.sort_unstable();
        let mut hash = Sha256::new();
        for (use_, key) in nodes {
            hash.update([use_ as u8]);
            hash.update((key.len() as u64).to_be_bytes());
            hash.update(key.as_bytes());
            hash.update(self.group(key).hash);
        }
        Planned {
            signature: hash.finalize().into(),
            certifiers: certifiers.into_iter().map(str::to_owned).collect(),
            reads: reads.into_iter().map(str::to_owned).collect(),
        }
    }
    /// For each group, a bound on how many levels deep SQLite can nest
    /// sub-programs while compiling a program that writes it. A trigger on
    /// a table nests the statements of its body, which write the groups it
    /// names; a foreign-key action nests a statement on the child, which
    /// for a CASCADE delete (or an action that is not a leaf) goes on to
    /// fire what the child's writes fire. SQLite codes each sub-program
    /// (a trigger or action, with one of six conflict policies) at most once
    /// per compile, so a path through a cycle passes each at most once.
    fn depths(&self) -> HashMap<String, u64> {
        let mut keys: Vec<&str> = self.groups.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let at: HashMap<&str, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let n = keys.len();
        let mut next = vec![Vec::new(); n];
        let mut leaves = vec![false; n];
        let mut programs = vec![0u64; n];
        let mut scratch = BTreeSet::new();
        for (index, key) in keys.iter().enumerate() {
            let group = self.group(key);
            programs[index] = 6 * group.triggers as u64;
            next[index].extend(
                self.resolved(key)
                    .1
                    .iter()
                    .filter_map(|t| at.get(t.as_str())),
            );
            for (child, position) in &group.into {
                let key = self.key(child, *position);
                if key.on_delete == Action::NoAction && key.on_update == Action::NoAction {
                    continue;
                }
                programs[index] += 2;
                let updates = |action| {
                    matches!(
                        action,
                        Action::SetNull | Action::SetDefault | Action::Cascade
                    )
                };
                let continues = matches!(key.on_delete, Action::Cascade | Action::Unknown)
                    || key.on_update == Action::Unknown
                    || ((matches!(key.on_delete, Action::SetNull | Action::SetDefault)
                        || updates(key.on_update))
                        && !self.leaf(child, &key.from, &mut scratch));
                if continues {
                    next[index].push(at[child.as_str()]);
                } else {
                    leaves[index] = true;
                }
            }
        }
        let mut depth = vec![0u64; n];
        for component in components(&next) {
            let members: HashSet<usize> = component.iter().copied().collect();
            let below = component
                .iter()
                .flat_map(|&g| &next[g])
                .filter(|t| !members.contains(t))
                .map(|&t| depth[t])
                .max();
            let value = match component.as_slice() {
                [g] if !next[*g].contains(g) => {
                    let mut value = u64::from(leaves[*g]);
                    if let Some(below) = below {
                        value = value.max(below.saturating_add(1));
                    }
                    value
                }
                _ => component
                    .iter()
                    .map(|&g| programs[g])
                    .fold(0u64, u64::saturating_add)
                    .saturating_add(below.unwrap_or(0)),
            };
            for &g in &component {
                depth[g] = value;
            }
        }
        keys.into_iter().map(str::to_owned).zip(depth).collect()
    }
}

/// A node of a signature's closure: a group written by the tree, or read.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Use {
    Written,
    Read,
}

/// The strongly connected components of a graph, each after every component
/// reachable from it (Tarjan's algorithm, iterative).
fn components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let (mut index, mut low, mut on_stack) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut found, mut next_index) = (Vec::new(), Vec::new(), 0);
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut work = vec![(start, 0usize)];
        index[start] = next_index;
        low[start] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&mut (node, ref mut edge)) = work.last_mut() {
            if let Some(&target) = edges[node].get(*edge) {
                *edge += 1;
                if index[target] == usize::MAX {
                    index[target] = next_index;
                    low[target] = next_index;
                    next_index += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    work.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(index[target]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                let mut component = Vec::new();
                loop {
                    let member = stack.pop().unwrap();
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                found.push(component);
            }
        }
    }
    found
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
