use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) const MAX_INPUT_BYTES: usize = 1_048_576;

/// Hash length-framed byte parts so concatenation boundaries cannot alias.
/// This is a content identity, not an authentication or authorization token.
pub(crate) fn digest(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hex::encode(hash.finalize())
}
/// Hash a serializable model with sorted object keys while preserving array order.
/// Caller-owned models must exclude credentials and other values not intended for a binding.
pub(crate) fn json_digest(value: &impl Serialize) -> String {
    fn sort(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.sort_keys();
                for value in map.values_mut() {
                    sort(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    sort(value);
                }
            }
            _ => (),
        }
    }
    let mut value = serde_json::to_value(value).expect("serializable migration model");
    sort(&mut value);
    digest(&[&serde_json::to_vec(&value).expect("serializable migration model")])
}

/// `json_digest` of successive catalogs that reuses each object's serialized
/// form while the object is unchanged. Replay fingerprints the whole catalog
/// after every migration; serializing every object again each time made that
/// quadratic in serialization work. Hashing still reads every byte.
#[derive(Default)]
pub(crate) struct CatalogDigests {
    objects: std::collections::HashMap<String, (serde_json::Value, Vec<u8>)>,
}
impl CatalogDigests {
    /// Exactly `json_digest(catalog)`.
    pub fn digest(&mut self, catalog: &super::model::Catalog) -> String {
        // `json_digest` sorts object keys; `serde_json` maps already iterate in
        // key order, as does the catalog's own map, so each object serializes
        // to the same bytes on its own.
        let mut previous = std::mem::take(&mut self.objects);
        for (key, value) in &catalog.objects {
            let entry = match previous.remove(key) {
                Some(entry) if entry.0 == *value => entry,
                _ => {
                    let mut bytes = serde_json::to_vec(key).expect("catalog keys serialize");
                    bytes.push(b':');
                    serde_json::to_writer(&mut bytes, value).expect("catalog values serialize");
                    (value.clone(), bytes)
                }
            };
            self.objects.insert(key.clone(), entry);
        }
        catalog_digest(
            catalog
                .objects
                .keys()
                .map(|key| self.objects[key].1.as_slice()),
            &catalog.uncovered,
        )
    }
}

/// `json_digest` of a catalog given each object's serialized `"key":value`
/// form, in key order, and its uncovered keys.
pub(crate) fn catalog_digest<'a>(
    objects: impl Iterator<Item = &'a [u8]> + Clone,
    uncovered: &[String],
) -> String {
    const OBJECTS: &[u8] = b"{\"objects\":{";
    const UNCOVERED: &[u8] = b"},\"uncovered\":[";
    const END: &[u8] = b"]}";
    let uncovered: Vec<Vec<u8>> = uncovered
        .iter()
        .map(|key| serde_json::to_vec(key).expect("catalog keys serialize"))
        .collect();
    let separated = |lengths: &mut dyn Iterator<Item = usize>| {
        lengths.fold((0usize, 0usize), |(total, n), length| {
            (total + length, n + 1)
        })
    };
    let (objects_length, objects_count) = separated(&mut objects.clone().map(<[u8]>::len));
    let (uncovered_length, uncovered_count) = separated(&mut uncovered.iter().map(Vec::len));
    let length = OBJECTS.len()
        + objects_length
        + objects_count.saturating_sub(1)
        + UNCOVERED.len()
        + uncovered_length
        + uncovered_count.saturating_sub(1)
        + END.len();
    let mut hash = Sha256::new();
    hash.update((length as u64).to_be_bytes());
    hash.update(OBJECTS);
    for (i, object) in objects.enumerate() {
        if i > 0 {
            hash.update(b",");
        }
        hash.update(object);
    }
    hash.update(UNCOVERED);
    for (i, key) in uncovered.iter().enumerate() {
        if i > 0 {
            hash.update(b",");
        }
        hash.update(key);
    }
    hash.update(END);
    hex::encode(hash.finalize())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Endpoint {
    pub connection_secret: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub role: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scratch {
    #[serde(flatten)]
    pub endpoint: Endpoint,
    pub disposable: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Database {
    pub engine: String,
    pub schema: String,
    pub migrations_dir: String,
    pub targets: BTreeMap<String, Endpoint>,
    pub scratch: Option<Scratch>,
    pub approval_secret: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adoption_checks: BTreeMap<String, String>,
    /// Absent serializes nothing, so the policy digest (and earlier plans)
    /// of a manifest without this table are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
}

const DEFAULT_STATEMENT_SECONDS: u64 = 90;
const DEFAULT_MIGRATOR_WAIT_SECONDS: u64 = 5;
/// A migration statement waits at most 1 second for each lock (the migration
/// `lock_timeout`); the whole transaction is then retried. The statement limit
/// counts from the start of the statement, so it must outlast that wait, or a
/// brief lock ends as a statement timeout, which is not retried.
/// `own_migrations` keeps its statement limit one second above its lock wait
/// in the same way.
pub(super) const MIN_STATEMENT_SECONDS: u64 = 2;
pub(super) const MIN_MIGRATOR_WAIT_SECONDS: u64 = 1;
/// A day: longer single statements belong in an operator-supervised build.
pub(super) const MAX_STATEMENT_SECONDS: u64 = 86_400;
/// Five minutes: a migrator holding the lock longer is stuck or building.
pub(super) const MAX_MIGRATOR_WAIT_SECONDS: u64 = 300;
fn default_statement_seconds() -> u64 {
    DEFAULT_STATEMENT_SECONDS
}
fn default_migrator_wait_seconds() -> u64 {
    DEFAULT_MIGRATOR_WAIT_SECONDS
}

/// `[databases.<name>.limits]` for `migrate` and `reconcile` on the target, in
/// whole seconds. PostgreSQL takes both fields; SQLite takes `statement_seconds`
/// for one migration script (#429). Omitted fields keep their defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Limits {
    /// Server limit for one migration script (each statement, with the whole
    /// script bounded 5 seconds later) or one concurrent index statement.
    #[serde(default = "default_statement_seconds")]
    pub statement_seconds: u64,
    /// How long to wait for another migrator's advisory lock.
    #[serde(default = "default_migrator_wait_seconds")]
    pub migrator_wait_seconds: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            statement_seconds: DEFAULT_STATEMENT_SECONDS,
            migrator_wait_seconds: DEFAULT_MIGRATOR_WAIT_SECONDS,
        }
    }
}
impl Limits {
    pub fn statement(&self) -> Duration {
        Duration::from_secs(self.statement_seconds)
    }
    pub fn migrator_wait(&self) -> Duration {
        Duration::from_secs(self.migrator_wait_seconds)
    }
    /// One run's work: a full migrator wait, a full statement and 5 seconds
    /// for everything else. Lowered limits never shorten it below 100 seconds.
    pub fn work_deadline(&self) -> Duration {
        Duration::from_secs(
            self.statement_seconds.max(DEFAULT_STATEMENT_SECONDS)
                + self
                    .migrator_wait_seconds
                    .max(DEFAULT_MIGRATOR_WAIT_SECONDS)
                + 5,
        )
    }
    /// The whole command: the work deadline plus a reserve in which fresh
    /// reconciliation after an interrupted COMMIT can still wait for the
    /// migrator lock (120 seconds with the defaults).
    pub fn total_deadline(&self) -> Duration {
        self.work_deadline()
            + Duration::from_secs(
                self.migrator_wait_seconds
                    .max(DEFAULT_MIGRATOR_WAIT_SECONDS)
                    + 15,
            )
    }
    /// Effective limits and the deadlines derived from them, for reports.
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "statement_seconds": self.statement_seconds,
            "migrator_wait_seconds": self.migrator_wait_seconds,
            "work_deadline_seconds": self.work_deadline().as_secs(),
            "total_deadline_seconds": self.total_deadline().as_secs(),
        })
    }
    fn validate(&self, database: &str, engine: &str, declares_wait: bool) -> Result<()> {
        let table = format!("[databases.{database}.limits]");
        // SQLite has no migrator advisory lock to wait for.
        if engine != "postgresql" && declares_wait {
            return Err(invalid(&format!(
                "{table} migrator_wait_seconds applies only to PostgreSQL; SQLite takes statement_seconds only."
            )));
        }
        for (field, value, min, max) in [
            (
                "statement_seconds",
                self.statement_seconds,
                MIN_STATEMENT_SECONDS,
                MAX_STATEMENT_SECONDS,
            ),
            (
                "migrator_wait_seconds",
                self.migrator_wait_seconds,
                MIN_MIGRATOR_WAIT_SECONDS,
                MAX_MIGRATOR_WAIT_SECONDS,
            ),
        ] {
            if !(min..=max).contains(&value) {
                return Err(invalid(&format!(
                    "{table} {field} must be from {min} to {max} seconds."
                )));
            }
        }
        Ok(())
    }
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    #[serde(default)]
    intent_ids: Vec<String>,
    #[serde(default)]
    requires: BTreeMap<String, String>,
    #[serde(default)]
    ensures: BTreeMap<String, String>,
    #[serde(default)]
    sqlite_rebuild: Option<String>,
}

// SQL/metadata are deliberately private and never Debug/Serialize: reports carry
// byte identities, not authored literals or provider credentials.
#[derive(Clone)]
pub(crate) struct Migration {
    pub id: u64,
    pub name: String,
    pub checksum: String,
    pub sql: String,
    pub metadata: Option<Vec<u8>>,
    pub intent_ids: Vec<String>,
    pub requires: BTreeMap<String, String>,
    pub ensures: BTreeMap<String, String>,
    pub sqlite_rebuild: Option<String>,
}
impl Migration {
    /// Expose source identities and traceability without raw SQL or sidecar contents.
    pub fn summary(&self) -> serde_json::Value {
        let mut summary = serde_json::json!({"id":self.id.to_string(), "source":self.name,
            "checksum":self.checksum, "sql_digest":digest(&[self.sql.as_bytes()]), "metadata_digest":self.metadata.as_ref().map(|b| digest(&[b])), "intent_ids":self.intent_ids, "requires":self.requires.keys().collect::<Vec<_>>(), "ensures":self.ensures.keys().collect::<Vec<_>>()});
        if let Some(table) = &self.sqlite_rebuild {
            summary["sqlite_rebuild"] = serde_json::json!(table);
        }
        summary
    }
}

pub(crate) struct CapturedInputs {
    pub config: Database,
    pub database_name: String,
    pub target_name: String,
    pub endpoint: Endpoint,
    pub document: toml::Value,
    pub migrations: Vec<Migration>,
    /// PostgreSQL: the schemas the captured history creates, inspected with
    /// the managed schema (see `analysis::history_schemas`). Empty for SQLite.
    pub history_schemas: Vec<String>,
    pub input_digest: String,
    pub policy_digest: String,
}

fn invalid(message: &str) -> Diagnostic {
    Diagnostic::new("DB_CONFIG", message)
}

// What to do about a refused entry of the migrations directory (#422). Each
// is about the directory, not the manifest.
const MOVE_OR_RENAME: &str = "Move the named entry out of the migrations directory, or rename it to <positive-id>_<name>.sql for a migration or to the .toml sidecar name of an existing migration. Names use ASCII letters, digits, underscores or hyphens and a lowercase suffix. Apart from README files and names starting with a dot, which are ignored, the directory holds nothing else. See docs/DATABASE_MIGRATIONS.md#immutable-sql-inventory.";
const RENUMBER: &str = "Rename whichever of the two named migrations has not been applied anywhere to an unused ID above the highest in the migrations directory (`ntnt db new <name>` takes the next free ID), and its .toml sidecar, if it has one, to the same new stem; never renumber an applied migration, and leave its sidecar as it is.";
const SAVE_AS_UTF8: &str = "Save the named file as UTF-8 text; the migrations directory holds only UTF-8 migrations and sidecars.";
const FIX_SIDECAR: &str = "Correct the named sidecar at the reported position. A sidecar holds only intent_ids = [\"<id>\", ...], [requires] and [ensures] tables that map check names to SQL strings, and, for SQLite, sqlite_rebuild = \"<table>\"; a migration without metadata needs no sidecar. See docs/DATABASE_MIGRATIONS.md#immutable-sql-inventory.";
const FIX_INTENT_IDS: &str =
    "Give each intent_ids entry in the named sidecar 1-128 characters and no control characters.";
const FIX_CHECKS: &str = "Rewrite each named check in the sidecar as one schema-qualified SELECT within the documented check subset, or remove it. Check names are simple ASCII identifiers, each check is at most 64 KiB and a migration has at most 64. See docs/DATABASE_MIGRATIONS.md#immutable-sql-inventory.";
const FIX_REBUILD: &str = "Remove sqlite_rebuild from the named sidecar, or set it to the one ordinary table (not a sqlite_ table) that the SQLite migration rebuilds; PostgreSQL migrations take no sqlite_rebuild. See docs/DATABASE_MIGRATIONS.md#sqlite-native-files-and-rebuilds.";
const ORPHAN: &str = "Add the matching .sql migration to the migrations directory, rename the sidecar to the stem of the migration it belongs to, or remove it.";
const DIRECTORY_LIMITS: &str = "Remove files that do not belong to this database's history from the migrations directory: capture reads at most 1,024 migrations, 2,048 files and 16 MiB. See docs/DATABASE_MIGRATIONS.md#immutable-sql-inventory.";
const UNLISTABLE: &str =
    "Make the migrations directory readable and listable by this user, then rerun.";

/// Entries of the migrations directory that are not part of its history and
/// are never read: names starting with `.` (`.gitkeep`, `.DS_Store`, editor
/// swap files and lock links, macOS `._` files) and README or README.<ext>
/// (one extension, such as README.md or readme.txt) in any letter case.
fn ignored_entry(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    name.starts_with('.')
        || name == "readme"
        || name
            .strip_prefix("readme.")
            .is_some_and(|extension| !extension.is_empty() && !extension.contains('.'))
}
/// Refuse an entry of the migrations directory (or the directory itself),
/// naming it in the message and in `source` with advice about the directory.
fn refuse_entry(code: &str, entry: &str, problem: &str, next_action: &str) -> Diagnostic {
    let mut d = Diagnostic::new(code, &format!("{entry}: {problem}"));
    d.source = Some(entry.to_owned());
    d.next_action = next_action.to_owned();
    d
}
/// A file name or path as diagnostics show it: as written, or quoted and
/// escaped when it is not UTF-8 or holds a character a terminal could
/// interpret.
pub(super) fn shown_os(text: &std::ffi::OsStr) -> String {
    match text.to_str() {
        Some(plain) if plain.escape_debug().eq(plain.chars()) => plain.to_owned(),
        _ => format!("{text:?}"),
    }
}
/// An entry of the migrations directory `dir` as diagnostics name it, relative
/// to the project: `migrations/0001_initial.sql`.
fn entry_label(dir: &str, name: &std::ffi::OsStr) -> String {
    format!("{dir}/{}", shown_os(name))
}
fn shown_entry(dir: &str, name: &str) -> String {
    entry_label(dir, std::ffi::OsStr::new(name))
}
/// A check name or metadata key as diagnostics show it: as written when it is
/// a simple name, otherwise quoted and escaped, at most 64 characters.
fn shown_key(key: &str) -> String {
    if !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return key.to_owned();
    }
    let mut bounded: String = key.chars().take(64).collect();
    if bounded.len() < key.len() {
        bounded.push('…');
    }
    format!("{bounded:?}")
}
/// The line and column (1-based, in characters) just after `text`.
fn line_column(text: &str) -> (usize, usize) {
    let line = 1 + text.matches('\n').count();
    let column = 1 + text.rsplit('\n').next().unwrap_or_default().chars().count();
    (line, column)
}
/// The position of the first byte of `bytes` that is not UTF-8.
fn invalid_utf8_position(bytes: &[u8], error: std::str::Utf8Error) -> String {
    let valid = std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default();
    let (line, column) = line_column(valid);
    format!("line {line}, column {column}")
}
/// The position of byte `offset` of `text` as toml's own messages give it:
/// the end of the input is one column past its last character.
fn toml_position(text: &str, offset: usize) -> String {
    let offset = (0..=offset.min(text.len()))
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or_default();
    let (line, column) = match text[..offset].char_indices().last() {
        Some((last, _)) if offset == text.len() => {
            let (line, column) = line_column(&text[..last]);
            (line, column + 1)
        }
        _ => line_column(&text[..offset]),
    };
    format!("line {line}, column {column}")
}
/// Parse a sidecar. A refusal names the sidecar, the position and what is
/// wrong, never what the file holds.
fn parse_metadata(sidecar: &str, bytes: &[u8]) -> Result<Metadata> {
    let refuse =
        |problem: String| refuse_entry("DB_METADATA_UNSUPPORTED", sidecar, &problem, FIX_SIDECAR);
    let text = std::str::from_utf8(bytes).map_err(|error| {
        refuse(format!(
            "metadata must be UTF-8; the first other byte is at {}.",
            invalid_utf8_position(bytes, error)
        ))
    })?;
    let at = |span: Option<std::ops::Range<usize>>| {
        toml_position(text, span.map_or(0, |span| span.start))
    };
    toml::from_str::<Metadata>(text).map_err(|error| match text.parse::<toml::Table>() {
        Err(syntax) => refuse(format!("not valid TOML at {}.", at(syntax.span()))),
        Ok(table) => refuse(format!(
            "unsupported metadata at {}: {}.",
            at(error.span()),
            metadata_problems(&table)
        )),
    })
}
/// What a sidecar that is valid TOML holds besides the supported fields.
fn metadata_problems(table: &toml::Table) -> String {
    let mut problems = Vec::new();
    for (key, value) in table {
        let (fits, expected) = match key.as_str() {
            "intent_ids" => (
                value
                    .as_array()
                    .is_some_and(|ids| ids.iter().all(toml::Value::is_str)),
                "intent_ids must be an array of strings".to_owned(),
            ),
            phase @ ("requires" | "ensures") => (
                value
                    .as_table()
                    .is_some_and(|checks| checks.values().all(toml::Value::is_str)),
                format!("[{phase}] must map each check name to an SQL string"),
            ),
            "sqlite_rebuild" => (
                value.is_str(),
                "sqlite_rebuild must be a table name".to_owned(),
            ),
            other => (false, format!("unknown key `{}`", shown_key(other))),
        };
        if !fits {
            problems.push(expected);
        }
    }
    if problems.is_empty() {
        "the fields do not match intent_ids, requires, ensures and sqlite_rebuild".into()
    } else {
        problems.join("; ")
    }
}
/// At most this many declared names are listed in a diagnostic.
const NAMES_SHOWN: usize = 20;
/// Declared manifest keys for a diagnostic: shown as `shown_key` shows them,
/// at most `NAMES_SHOWN`, or `none`.
fn declared_names<'a>(names: impl ExactSizeIterator<Item = &'a String>) -> String {
    let total = names.len();
    let mut shown: Vec<String> = names
        .take(NAMES_SHOWN)
        .map(|name| shown_key(name))
        .collect();
    if total > NAMES_SHOWN {
        shown.push(format!("and {} more", total - NAMES_SHOWN));
    }
    if shown.is_empty() {
        "none".to_owned()
    } else {
        shown.join(", ")
    }
}
/// What is wrong with checks outside the supported subset, naming each (with
/// its phase when there are several), or `None` when every check is supported.
fn unsupported_checks(
    phases: &[(&str, &BTreeMap<String, String>)],
    schema: &str,
) -> Option<String> {
    let several = phases.len() > 1;
    let count: usize = phases.iter().map(|(_, checks)| checks.len()).sum();
    if count > 64 {
        return Some(format!("{count} checks; at most 64 are allowed."));
    }
    let named: Vec<String> = phases
        .iter()
        .flat_map(|(phase, checks)| {
            checks
                .iter()
                .filter(|(name, sql)| {
                    !identifier(name)
                        || sql.len() > 65536
                        || !super::analysis::contract_supported(sql, schema)
                })
                .map(move |(name, _)| match several {
                    true => format!("{} ([{phase}])", shown_key(name)),
                    false => shown_key(name),
                })
        })
        .collect();
    match named.as_slice() {
        [] => None,
        [one] => Some(format!(
            "check {one} is outside the supported check subset; unsupported SQL is never executed."
        )),
        many => Some(format!(
            "checks {} are outside the supported check subset; unsupported SQL is never executed.",
            many.join(", ")
        )),
    }
}
pub(crate) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && s.as_bytes()[0].is_ascii_alphabetic()
}
/// An input file, and how its diagnostics name it: as the user gave it, or,
/// for an entry of the migrations directory, relative to the project.
#[derive(Clone, Copy)]
pub(super) enum Input<'a> {
    /// The manifest (`--manifest`, ntnt.toml by default).
    Manifest(&'a Path),
    /// A saved plan, given with `flag` (`--plan` or `--against`).
    Plan(&'a Path, &'static str),
    /// An entry of the migrations directory at a path, named as shown.
    Entry(&'a Path, &'a str),
}
/// Why an input file could not be captured (#422).
enum Unreadable {
    Missing,
    Symlink,
    NotRegular,
    TooLarge,
    Linked,
    Changed,
    Io(std::io::Error),
}
impl Input<'_> {
    fn path(&self) -> &Path {
        match self {
            Self::Manifest(path) | Self::Plan(path, _) | Self::Entry(path, _) => path,
        }
    }
    /// Refuse the file for `cause`, naming it in the message and `source`,
    /// with advice for this kind of input.
    fn refuse(&self, cause: Unreadable) -> Diagnostic {
        use Unreadable::*;
        let shown = match self {
            Self::Manifest(path) | Self::Plan(path, _) => shown_os(path.as_os_str()),
            Self::Entry(_, shown) => (*shown).to_owned(),
        };
        let reason = match &cause {
            Missing => "not found".to_owned(),
            Symlink => "a symbolic link; `ntnt db` reads only regular files".to_owned(),
            NotRegular => "not a regular file".to_owned(),
            TooLarge => "larger than 1 MiB, the limit for one input file".to_owned(),
            Linked => {
                "more than one hard link; `ntnt db` reads only files with a single link".to_owned()
            }
            Changed => "changed while it was being read".to_owned(),
            Io(error) => format!("cannot be read ({error})"),
        };
        let (message, next_action) = match self {
            Self::Manifest(path) => {
                let message = match cause {
                    Missing => {
                        // A bare file name is looked up in the current directory only.
                        let here = if path.components().count() == 1 {
                            " in the current directory"
                        } else {
                            ""
                        };
                        format!("Manifest {shown}: not found{here}; run from the project directory or pass --manifest <path>.")
                    }
                    _ => format!("Manifest {shown}: {reason}."),
                };
                let next_action = match cause {
                    Missing => "Run the command from the project directory that holds ntnt.toml, or pass --manifest <path> naming that file; `ntnt db` does not search parent directories. `ntnt db init` creates a manifest.",
                    Symlink => "Pass --manifest with the path of the file the link points to, or replace the link with that file.",
                    NotRegular => "Pass --manifest with the path of the project's ntnt.toml file, not a directory or special file.",
                    TooLarge => "Keep the manifest under 1 MiB: move large configuration out of it.",
                    Linked => "Give the manifest a single link: copy it to a new name, then rename the copy over it.",
                    Changed => "Rerun once nothing is changing the manifest, such as an editor saving it or another `ntnt db init` or `ntnt db new`.",
                    Io(_) => "Make the manifest readable by this user, then rerun. On Windows, rerun once a running `ntnt db init` or `ntnt db new` has finished: it locks the manifest while it works.",
                };
                (message, next_action.to_owned())
            }
            Self::Plan(_, flag) => (
                format!("{flag} file {shown}: {reason}."),
                match cause {
                    Missing => format!("Pass {flag} with the path of a plan saved by `ntnt db plan --out <file>`."),
                    Symlink => format!("Pass {flag} with the path of the saved plan itself, not a link to it."),
                    NotRegular => format!("Pass {flag} with the path of a saved plan file, not a directory or special file."),
                    TooLarge => format!("Pass {flag} with a plan saved by `ntnt db plan --out`; saved plans are never larger than 1 MiB."),
                    Linked => format!("Copy the saved plan to a new name, which has a single link, and pass that to {flag}."),
                    Changed => "Rerun once nothing is writing the saved plan.".to_owned(),
                    Io(_) => "Make the saved plan readable by this user, then rerun.".to_owned(),
                },
            ),
            Self::Entry(..) => (
                format!("{shown}: {reason}."),
                match cause {
                    Missing => "Rerun once nothing is changing the migrations directory.",
                    Symlink => "Replace the link with a copy of the file it points to, or remove it from the migrations directory.",
                    NotRegular => "Move the named entry out of the migrations directory, which holds only regular files.",
                    TooLarge => "Split the named migration into migrations of at most 1 MiB each; the migrations directory holds no larger file.",
                    Linked => "Give the named entry a single link: copy it to a new name outside the migrations directory, then move the copy over it.",
                    Changed => "Rerun once nothing is writing the migrations directory.",
                    Io(_) => "Make the named entry in the migrations directory readable by this user, then rerun.",
                }
                .to_owned(),
            ),
        };
        let mut d = Diagnostic::new("DB_INPUT_IO", &message);
        d.source = Some(shown);
        d.next_action = next_action;
        d
    }
}
/// Capture one bounded regular file and reject links or detected replacement/mutation.
/// Callers retain these bytes instead of reopening a validated path during execution.
pub(super) fn file_bytes(input: Input) -> Result<Vec<u8>> {
    Ok(capture(input, super::input_file::open, |_| Ok(()))?.1)
}
/// `file_bytes` through `open`, running `before_read` on the opened handle (`ntnt db
/// init` locks the manifest there) and returning the handle with the bytes. A
/// refusal names the file and says which rule it broke.
pub(super) fn capture(
    input: Input,
    open: fn(&Path) -> std::io::Result<File>,
    before_read: impl FnOnce(&File) -> Result<()>,
) -> Result<(File, Vec<u8>)> {
    use Unreadable::*;
    let path = input.path();
    let refuse = |cause| input.refuse(cause);
    let missing_or = |error: std::io::Error| match error.kind() {
        std::io::ErrorKind::NotFound => Missing,
        _ => Io(error),
    };
    let meta = std::fs::symlink_metadata(path).map_err(|e| refuse(missing_or(e)))?;
    if meta.file_type().is_symlink() {
        return Err(refuse(Symlink));
    }
    if !meta.is_file() {
        return Err(refuse(NotRegular));
    }
    if meta.len() > MAX_INPUT_BYTES as u64 {
        return Err(refuse(TooLarge));
    }
    let file = open(path).map_err(|e| refuse(missing_or(e)))?;
    before_read(&file)?;
    let opened = file.metadata().map_err(|e| refuse(Io(e)))?;
    // A regular file a moment ago: anything else now replaced it.
    if !opened.is_file() || opened.file_type().is_symlink() {
        return Err(refuse(Changed));
    }
    if !super::input_file::single_link(&file, &opened) {
        return Err(refuse(Linked));
    }
    let mut bytes = Vec::new();
    (&file)
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| refuse(Io(e)))?;
    let after = std::fs::symlink_metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => refuse(Changed),
        _ => refuse(Io(e)),
    })?;
    let finished = file.metadata().map_err(|e| refuse(Io(e)))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(refuse(TooLarge));
    }
    if !super::input_file::same(&meta, &opened)
        || !super::input_file::same(&opened, &after)
        || !super::input_file::same(&opened, &finished)
    {
        return Err(refuse(Changed));
    }
    if !super::input_file::single_link(&file, &finished) {
        return Err(refuse(Linked));
    }
    Ok((file, bytes))
}
/// The checksum an adopted checkpoint records for the migrations it covers,
/// given as their `Migration::summary`s in order.
pub(super) fn baseline_checksum(prefix: &[serde_json::Value]) -> String {
    json_digest(&serde_json::json!({"format":"ntnt-baseline-v1","prefix":prefix}))
}

impl CapturedInputs {
    /// Effective execution limits: the declared table or the defaults.
    pub fn limits(&self) -> Limits {
        self.config.limits.unwrap_or_default()
    }
    /// The limits a plan binds. PostgreSQL binds its effective limits. SQLite
    /// binds `statement_seconds` only when the manifest declares the table, so
    /// plans saved without one keep their digests (#429).
    pub fn plan_limits(&self) -> Option<serde_json::Value> {
        if self.config.engine == "postgresql" {
            return Some(serde_json::json!(self.limits()));
        }
        self.config
            .limits
            .map(|limits| serde_json::json!({ "statement_seconds": limits.statement_seconds }))
    }
    /// Bind the exact covered source prefix without claiming its SQL was executed.
    pub fn checkpoint(&self, count: usize) -> Result<super::model::HistoryEntry> {
        let last = count
            .checked_sub(1)
            .and_then(|n| self.migrations.get(n))
            .ok_or_else(|| {
                Diagnostic::new(
                    "DB_ADOPTION_INPUT",
                    "Adoption requires a nonempty captured migration prefix.",
                )
            })?;
        let prefix: Vec<_> = self
            .migrations
            .iter()
            .take(count)
            .map(Migration::summary)
            .collect();
        Ok(super::model::HistoryEntry {
            position: count as i64,
            id: last.id.to_string(),
            name: last.name.clone(),
            checksum: baseline_checksum(&prefix),
            state: "adopted".into(),
        })
    }
    /// Parse captured manifest bytes; invalid UTF-8 or TOML is a configuration error.
    pub(super) fn parse(bytes: &[u8]) -> Result<toml::Value> {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid("Invalid UTF-8/TOML manifest."))
    }
    /// Select exactly one named logical database and validate its definition.
    /// `database` may be omitted only when exactly one is declared. Only the selected
    /// definition is interpreted; no directory, target or secret is accessed here.
    pub(super) fn select(
        document: &toml::Value,
        database: Option<&str>,
    ) -> Result<(String, Database)> {
        if document.get("database").is_some() {
            return Err(invalid("Use named [databases.<name>] tables instead of the prototype [database] table; the two forms cannot be mixed."));
        }
        let databases = document
            .get("databases")
            .and_then(toml::Value::as_table)
            .filter(|databases| !databases.is_empty())
            .ok_or_else(|| invalid("Declare at least one [databases.<name>] table."))?;
        if let Some((name, _)) = databases.iter().find(|(name, value)| {
            name.is_empty()
                || name.len() > 63
                || !name.as_bytes()[0].is_ascii_alphabetic()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !value.is_table()
        }) {
            // Identify the key, never the associated value. Bound and escape
            // malformed names so an error cannot inject terminal control text.
            let mut label: String = name.chars().take(63).collect();
            if label.len() < name.len() {
                label.push('…');
            }
            return Err(invalid(&format!("Invalid database entry databases.{label:?}: names must start with an ASCII letter and contain at most 63 letters, digits, underscores or hyphens; each entry must be a table.")));
        }
        // The declared names are manifest keys, validated above; never secrets.
        let declared = declared_names(databases.keys());
        let choices = databases
            .keys()
            .take(NAMES_SHOWN)
            .map(|name| format!("--database {name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let database_name = match database {
            Some(name) if databases.contains_key(name) => name.to_owned(),
            Some(name) => {
                let mut d = Diagnostic::new(
                    "DB_DATABASE_UNKNOWN",
                    &format!(
                        "Database {} is not declared; declared databases: {declared}. No other database was selected.",
                        shown_key(name)
                    ),
                );
                d.next_action = format!("Pass one of the declared databases ({choices}), or declare the database as a [databases.<name>] table in the manifest.");
                return Err(d);
            }
            None if databases.len() == 1 => databases.keys().next().unwrap().clone(),
            None => {
                let mut d = Diagnostic::new(
                    "DB_DATABASE_REQUIRED",
                    &format!("Several logical databases are declared ({declared}); select one with --database before choosing its target."),
                );
                d.next_action = format!("Pass one of {choices}; --database may be omitted only when one logical database is declared.");
                return Err(d);
            }
        };
        // Only the selected definition is interpreted. Other stores may use a
        // future adapter, unavailable credentials, or unprovisioned directories.
        let raw = &databases[&database_name];
        // The configuration is tables. Serde's derived structs would also accept
        // arrays by position (`limits = [1, 300]`), so reject any other shape.
        let table = |value: Option<&toml::Value>| value.is_none_or(toml::Value::is_table);
        if !raw.get("limits").is_none_or(toml::Value::is_table) {
            return Err(invalid(&format!("[databases.{database_name}.limits] must be a table of statement_seconds and migrator_wait_seconds, each a whole number of seconds.")));
        }
        if !raw.is_table()
            || !table(raw.get("scratch"))
            || !raw.get("targets").is_none_or(|targets| {
                targets
                    .as_table()
                    .is_some_and(|targets| targets.values().all(toml::Value::is_table))
            })
        {
            return Err(invalid(
                "Database, target and scratch definitions must be TOML tables.",
            ));
        }
        let config: Database = raw.clone().try_into().map_err(|_| {
            // Name the limits table when it is the culprit, never its values.
            if raw
                .get("limits")
                .is_some_and(|limits| limits.clone().try_into::<Limits>().is_err())
            {
                invalid(&format!("[databases.{database_name}.limits] accepts only statement_seconds and migrator_wait_seconds, each a whole number of seconds."))
            } else {
                invalid("Unknown or invalid database configuration field.")
            }
        })?;
        // NTNT's own schemas: the ledger, and the replay marker that
        // replay creates in scratch before the managed schema.
        if config.engine == "postgresql"
            && ["_ntnt", "_ntnt_replay"].contains(&config.schema.as_str())
        {
            return Err(invalid(&format!(
                "[databases.{database_name}] schema = \"{}\" names a schema NTNT reserves: _ntnt holds its migration ledger and _ntnt_replay marks a replay in the scratch database, so neither can be the managed schema. Use `public` or another schema.",
                config.schema
            )));
        }
        match config.engine.as_str() {
            "postgresql"
                if identifier(&config.schema)
                    && config.schema != "information_schema"
                    && !config.schema.starts_with("pg_") => {}
            "sqlite" if config.schema == "main" => (),
            _ => return Err(invalid(
                "Use PostgreSQL with `public` or another non-system schema, or SQLite with schema=main.",
            )),
        }
        if let Some(limits) = &config.limits {
            let declares_wait = raw
                .get("limits")
                .and_then(|limits| limits.get("migrator_wait_seconds"))
                .is_some();
            limits.validate(&database_name, &config.engine, declares_wait)?;
        }
        let adoption_checks = [("adoption_checks", &config.adoption_checks)];
        if let Some(problem) = unsupported_checks(&adoption_checks, &config.schema) {
            return Err(Diagnostic::new(
                "DB_CHECK_UNSUPPORTED",
                &format!("[databases.{database_name}.adoption_checks]: {problem}"),
            ));
        }
        Ok((database_name, config))
    }
    /// Resolve the selected target and validate its endpoint, scratch and approval
    /// references. Secret names are validated here; their values are never read.
    pub(super) fn endpoint(config: &Database, database: &str, target: &str) -> Result<Endpoint> {
        let endpoint = config.targets.get(target).cloned().ok_or_else(|| {
            // Target names are manifest keys, shown escaped; never secrets.
            let declared = declared_names(config.targets.keys());
            let mut d = invalid(&format!(
                "Target {} is not declared for database {database}; declared targets: {declared}.",
                shown_key(target)
            ));
            d.next_action = if config.targets.is_empty() {
                format!("Declare a [databases.{database}.targets.<name>] table with its connection_secret in the manifest, then pass --target <name>.")
            } else {
                let choices = config
                    .targets
                    .keys()
                    .take(NAMES_SHOWN)
                    .map(|name| format!("--target {}", shown_key(name)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Pass one of the declared targets ({choices}); names are case-sensitive. To add a target, declare [databases.{database}.targets.<name>] with its connection_secret in the manifest.")
            };
            d
        })?;
        for ep in std::iter::once(&endpoint).chain(config.scratch.iter().map(|s| &s.endpoint)) {
            if (config.engine == "postgresql"
                && (!identifier(&ep.database) || !identifier(&ep.role)))
                || (config.engine == "sqlite" && (!ep.database.is_empty() || !ep.role.is_empty()))
                || crate::secret::validate_secret_name(&ep.connection_secret).is_err()
            {
                return Err(invalid("Database/role names must be simple identifiers and connections must use logical secret references."));
            }
        }
        if config.scratch.as_ref().is_some_and(|s| {
            !s.disposable || s.endpoint.connection_secret == endpoint.connection_secret
        }) {
            return Err(invalid(
                "Scratch must be explicitly disposable with its own secret reference.",
            ));
        }
        if config.approval_secret.as_ref().is_some_and(|name| {
            crate::secret::validate_secret_name(name).is_err()
                || config
                    .targets
                    .values()
                    .any(|e| &e.connection_secret == name)
                || config
                    .scratch
                    .as_ref()
                    .is_some_and(|s| &s.endpoint.connection_secret == name)
        }) {
            return Err(invalid(
                "Approval must use a separately named secret, never a connection secret.",
            ));
        }
        Ok(endpoint)
    }
    /// Resolve the confined migration directory relative to the manifest. Every
    /// component must already exist as a directory, never a link.
    pub(super) fn migrations_dir(manifest: &Path, config: &Database) -> Result<PathBuf> {
        // Portable lexical confinement; never reinterpret backslashes/drives on a different OS.
        if config.migrations_dir.is_empty()
            || config.migrations_dir.contains(['\\', ':', '\0'])
            || config
                .migrations_dir
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(invalid("migrations_dir must be a confined relative path."));
        }
        let root = manifest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .map_err(|_| invalid("Cannot resolve project root."))?;
        let mut dir = root.clone();
        for part in config.migrations_dir.split('/') {
            dir.push(part);
            let md = std::fs::symlink_metadata(&dir)
                .map_err(|_| invalid("Missing migration directory."))?;
            if !md.is_dir() || md.file_type().is_symlink() {
                return Err(invalid("Migration directories cannot be links."));
            }
        }
        Ok(dir)
    }
    /// Capture the manifest and exactly one logical database's ordered migration inputs.
    /// Ambiguous selection and malformed registry entries fail before credential lookup;
    /// unselected stores' directories are not visited. Exact bytes and logical selection
    /// participate in plan identity, while secret resolution remains a later operation.
    pub fn load(manifest: &Path, target: &str, database: Option<&str>) -> Result<Self> {
        let bytes = file_bytes(Input::Manifest(manifest))?;
        let document = Self::parse(&bytes)?;
        let (database_name, config) = Self::select(&document, database)?;
        let endpoint = Self::endpoint(&config, &database_name, target)?;
        let dir = Self::migrations_dir(manifest, &config)?;
        // Refusals name entries relative to the project, through the
        // directory as the manifest names it (#422).
        let shown = config.migrations_dir.as_str();
        let unlistable = |error: std::io::Error| {
            refuse_entry(
                "DB_CONFIG",
                shown,
                &format!("cannot list the migrations directory ({error})."),
                UNLISTABLE,
            )
        };
        let mut sources = BTreeMap::new();
        let mut total_bytes = 0usize;
        for entry in std::fs::read_dir(&dir).map_err(unlistable)? {
            let entry = entry.map_err(unlistable)?;
            let file_name = entry.file_name();
            if ignored_entry(&file_name) {
                continue;
            }
            let label = entry_label(shown, &file_name);
            let name = file_name.into_string().map_err(|_| {
                refuse_entry(
                    "DB_CONFIG",
                    &label,
                    "migration file names must be UTF-8.",
                    MOVE_OR_RENAME,
                )
            })?;
            if !(name.ends_with(".sql") || name.ends_with(".toml")) {
                return Err(refuse_entry(
                    "DB_CONFIG",
                    &label,
                    "the migrations directory holds only <id>_<name>.sql migrations and their matching .toml sidecars.",
                    MOVE_OR_RENAME,
                ));
            }
            if sources.len() >= 2048 {
                return Err(refuse_entry(
                    "DB_CONFIG",
                    shown,
                    "more than 2,048 files (at most 1,024 migrations and their sidecars).",
                    DIRECTORY_LIMITS,
                ));
            }
            let bytes = file_bytes(Input::Entry(&entry.path(), &label))?;
            total_bytes += bytes.len();
            if total_bytes > 16 * 1_048_576 {
                return Err(refuse_entry(
                    "DB_CONFIG",
                    shown,
                    "the migration files exceed 16 MiB in total.",
                    DIRECTORY_LIMITS,
                ));
            }
            sources.insert(name, bytes);
        }
        let mut ids = BTreeMap::new();
        let mut migrations = Vec::new();
        for (name, sql_bytes) in sources.iter().filter(|(name, _)| name.ends_with(".sql")) {
            let label = shown_entry(shown, name);
            let misnamed =
                |problem: &str| refuse_entry("DB_CONFIG", &label, problem, MOVE_OR_RENAME);
            let stem = name.strip_suffix(".sql").unwrap();
            let (number, description) = stem
                .split_once('_')
                .filter(|(_, description)| !description.is_empty())
                .ok_or_else(|| misnamed("migration file names are <numeric-id>_<name>.sql."))?;
            let id = number
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0 && number.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| misnamed("the migration ID must be a positive decimal integer."))?;
            if !description
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(misnamed(
                    "migration names use ASCII letters, digits, underscores or hyphens.",
                ));
            }
            if let Some(first) = ids.insert(id, name) {
                return Err(refuse_entry(
                    "DB_DUPLICATE_ID",
                    &label,
                    &format!(
                        "migration ID {id} is also the ID of {}.",
                        shown_entry(shown, first)
                    ),
                    RENUMBER,
                ));
            }
            let sql = String::from_utf8(sql_bytes.clone()).map_err(|error| {
                refuse_entry(
                    "DB_CONFIG",
                    &label,
                    &format!(
                        "SQL must be UTF-8; the first other byte is at {}.",
                        invalid_utf8_position(sql_bytes, error.utf8_error())
                    ),
                    SAVE_AS_UTF8,
                )
            })?;
            let sidecar = shown_entry(shown, &format!("{stem}.toml"));
            let metadata = sources.get(&format!("{stem}.toml")).cloned();
            let parsed: Metadata = match &metadata {
                Some(bytes) => parse_metadata(&sidecar, bytes)?,
                None => Metadata::default(),
            };
            if parsed
                .intent_ids
                .iter()
                .any(|s| s.len() > 128 || s.is_empty() || s.chars().any(char::is_control))
            {
                return Err(refuse_entry(
                    "DB_CONFIG",
                    &sidecar,
                    "each intent_ids entry needs 1-128 characters and no control characters.",
                    FIX_INTENT_IDS,
                ));
            }
            let phases = [("requires", &parsed.requires), ("ensures", &parsed.ensures)];
            if let Some(problem) = unsupported_checks(&phases, &config.schema) {
                return Err(refuse_entry(
                    "DB_CHECK_UNSUPPORTED",
                    &sidecar,
                    &problem,
                    FIX_CHECKS,
                ));
            }
            if parsed.sqlite_rebuild.as_ref().is_some_and(|table| {
                config.engine != "sqlite"
                    || !identifier(table)
                    || table.to_ascii_lowercase().starts_with("sqlite_")
            }) {
                return Err(refuse_entry(
                    "DB_METADATA_UNSUPPORTED",
                    &sidecar,
                    "sqlite_rebuild requires one ordinary table name and the SQLite adapter.",
                    FIX_REBUILD,
                ));
            }
            let checksum = digest(&[
                b"ntnt-migration-v1",
                name.as_bytes(),
                sql_bytes,
                if metadata.is_some() {
                    b"sidecar"
                } else {
                    b"absent"
                },
                metadata.as_deref().unwrap_or_default(),
            ]);
            migrations.push(Migration {
                id,
                name: name.clone(),
                checksum,
                sql,
                metadata,
                intent_ids: parsed.intent_ids,
                requires: parsed.requires,
                ensures: parsed.ensures,
                sqlite_rebuild: parsed.sqlite_rebuild,
            });
        }
        for name in sources.keys().filter(|n| n.ends_with(".toml")) {
            let sql = format!("{}.sql", name.strip_suffix(".toml").unwrap());
            if !sources.contains_key(&sql) {
                return Err(refuse_entry(
                    "DB_CONFIG",
                    &shown_entry(shown, name),
                    &format!(
                        "no migration {} matches this sidecar.",
                        shown_entry(shown, &sql)
                    ),
                    ORPHAN,
                ));
            }
        }
        if migrations.len() > 1024 {
            return Err(refuse_entry(
                "DB_CONFIG",
                shown,
                "more than 1,024 migrations.",
                DIRECTORY_LIMITS,
            ));
        }
        migrations.sort_by_key(|m| m.id);
        let summaries: Vec<_> = migrations.iter().map(Migration::summary).collect();
        let policy_digest = json_digest(
            &serde_json::json!({"policy":if config.engine=="sqlite" {"exact-plan/transactional-sqlite-v1"} else {"exact-plan/transactional-postgresql-v1"}, "database_name":database_name, "database":config}),
        );
        let input_digest = digest(&[
            b"ntnt-inputs-v1",
            &bytes,
            database_name.as_bytes(),
            json_digest(&summaries).as_bytes(),
        ]);
        let history_schemas = if config.engine == "postgresql" {
            super::analysis::history_schemas(
                migrations.iter().map(|m| m.sql.as_str()),
                &config.schema,
            )
        } else {
            Vec::new()
        };
        Ok(Self {
            config,
            database_name,
            target_name: target.into(),
            endpoint,
            document,
            migrations,
            history_schemas,
            input_digest,
            policy_digest,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("migrations")).unwrap();
        std::fs::write(
            root.path().join("ntnt.toml"),
            r#"
[databases.app]
engine="postgresql"
schema="app"
migrations_dir="migrations"
[databases.app.targets.local]
connection_secret="TEST_URL"
database="target"
role="target_role"
[secrets.TEST_URL]
"#,
        )
        .unwrap();
        root
    }
    /// NTNT's own schemas cannot be the managed schema: `_ntnt` holds the
    /// ledger, and replay creates `_ntnt_replay` in scratch before the
    /// managed schema, so a plan would fail on a duplicate schema.
    #[test]
    fn ntnt_schemas_are_refused_as_the_managed_schema() {
        let root = project();
        let manifest = root.path().join("ntnt.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        for schema in ["_ntnt", "_ntnt_replay"] {
            std::fs::write(
                &manifest,
                text.replace("schema=\"app\"", &format!("schema=\"{schema}\"")),
            )
            .unwrap();
            let Err(error) = CapturedInputs::load(&manifest, "local", None) else {
                panic!("{schema} was accepted as the managed schema");
            };
            assert_eq!(error.code, "DB_CONFIG", "{schema}");
            assert!(
                error.message.contains(&format!(
                    "[databases.app] schema = \"{schema}\" names a schema NTNT reserves"
                )),
                "{}",
                error.message
            );
        }
        for schema in ["public", "app", "ntnt", "ntnt_replay"] {
            std::fs::write(
                &manifest,
                text.replace("schema=\"app\"", &format!("schema=\"{schema}\"")),
            )
            .unwrap();
            if let Err(error) = CapturedInputs::load(&manifest, "local", None) {
                panic!("{schema}: {}", error.message);
            }
        }
    }
    fn with_limits(root: &tempfile::TempDir, limits: &str) -> std::path::PathBuf {
        let manifest = root.path().join("ntnt.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            format!("{text}[databases.app.limits]\n{limits}\n"),
        )
        .unwrap();
        manifest
    }
    fn sqlite_project(limits: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("migrations")).unwrap();
        let limits = limits.map_or(String::new(), |l| format!("[databases.app.limits]\n{l}\n"));
        std::fs::write(
            root.path().join("ntnt.toml"),
            format!("[databases.app]\nengine = \"sqlite\"\nschema = \"main\"\nmigrations_dir = \"migrations\"\n[databases.app.targets.local]\nconnection_secret = \"DB_TARGET\"\n{limits}[secrets.DB_TARGET]\n"),
        )
        .unwrap();
        root
    }
    #[test]
    fn sqlite_takes_statement_seconds_and_binds_it_only_when_declared() {
        let load = |root: &tempfile::TempDir| {
            CapturedInputs::load(&root.path().join("ntnt.toml"), "local", None)
        };
        // Without the table nothing is bound, so earlier SQLite plans keep their digests.
        let absent = load(&sqlite_project(None)).unwrap();
        assert_eq!(absent.plan_limits(), None);
        assert_eq!(absent.limits().statement().as_secs(), 90);
        let declared = load(&sqlite_project(Some("statement_seconds = 600"))).unwrap();
        assert_eq!(
            declared.plan_limits(),
            Some(serde_json::json!({"statement_seconds": 600}))
        );
        assert_eq!(declared.limits().statement().as_secs(), 600);
        assert_ne!(declared.policy_digest, absent.policy_digest);
        // SQLite has no migrator advisory lock to wait for.
        let Err(error) = load(&sqlite_project(Some("migrator_wait_seconds = 10"))) else {
            panic!("SQLite accepted migrator_wait_seconds");
        };
        assert_eq!(error.code, "DB_CONFIG");
        assert!(
            error
                .message
                .contains("migrator_wait_seconds applies only to PostgreSQL"),
            "{}",
            error.message
        );
        let Err(error) = load(&sqlite_project(Some("statement_seconds = 1"))) else {
            panic!("SQLite accepted statement_seconds = 1");
        };
        assert!(
            error
                .message
                .contains("statement_seconds must be from 2 to 86400"),
            "{}",
            error.message
        );
    }
    #[test]
    fn absent_limits_keep_the_defaults_and_the_earlier_digests() {
        let root = project();
        std::fs::write(
            root.path().join("migrations/1_first.sql"),
            "CREATE TABLE app.t(id INT);",
        )
        .unwrap();
        let inputs = CapturedInputs::load(&root.path().join("ntnt.toml"), "local", None).unwrap();
        // Digests of these exact inputs before limits existed (0.6.2 cycle
        // base): a manifest without limits keeps its saved plans valid.
        assert_eq!(
            inputs.policy_digest,
            "eab5d51ca9c9f49f7d081a47890d7c3d5c5eb6bc98c3220c489e7da550630114"
        );
        assert_eq!(
            inputs.input_digest,
            "e752df6db4a5ef75d90050e87f98a7ec6b5932647dfeef02dc5d74447a5ec693"
        );
        assert!(inputs.config.limits.is_none());
        assert_eq!(
            inputs.limits().report(),
            serde_json::json!({"statement_seconds":90,"migrator_wait_seconds":5,
                "work_deadline_seconds":100,"total_deadline_seconds":120})
        );
        assert_eq!(
            inputs.plan_limits(),
            Some(serde_json::json!({"statement_seconds":90,"migrator_wait_seconds":5}))
        );
    }
    #[test]
    fn declared_limits_extend_the_deadlines_and_change_the_policy() {
        let mut digests = BTreeSet::new();
        digests.insert(
            CapturedInputs::load(&project().path().join("ntnt.toml"), "local", None)
                .unwrap()
                .policy_digest,
        );
        for (limits, effective, work, total) in [
            ("statement_seconds = 3600", (3600, 5), 3610, 3630),
            ("statement_seconds = 3601", (3601, 5), 3611, 3631),
            ("migrator_wait_seconds = 60", (90, 60), 155, 230),
            (
                "statement_seconds = 86400\nmigrator_wait_seconds = 300",
                (86400, 300),
                86705,
                87020,
            ),
            // The lowest limits keep the default deadlines.
            (
                "statement_seconds = 2\nmigrator_wait_seconds = 1",
                (2, 1),
                100,
                120,
            ),
            // An empty table states the defaults explicitly.
            ("", (90, 5), 100, 120),
        ] {
            let root = project();
            let manifest = with_limits(&root, limits);
            let inputs = CapturedInputs::load(&manifest, "local", None).unwrap();
            let l = inputs.limits();
            assert_eq!(
                (l.statement_seconds, l.migrator_wait_seconds),
                effective,
                "{limits}"
            );
            assert_eq!(l.statement().as_secs(), effective.0);
            assert_eq!(l.migrator_wait().as_secs(), effective.1);
            assert_eq!(l.work_deadline().as_secs(), work, "{limits}");
            assert_eq!(l.total_deadline().as_secs(), total, "{limits}");
            assert_eq!(
                inputs.policy_digest,
                CapturedInputs::load(&manifest, "local", None)
                    .unwrap()
                    .policy_digest
            );
            assert!(digests.insert(inputs.policy_digest), "{limits}");
        }
    }
    #[test]
    fn invalid_limits_name_the_field_without_echoing_values() {
        for (limits, field) in [
            (
                "statement_seconds = 0",
                "statement_seconds must be from 2 to 86400",
            ),
            // One second would end a brief lock wait as a statement timeout
            // before the 1-second lock timeout could retry it.
            (
                "statement_seconds = 1",
                "statement_seconds must be from 2 to 86400",
            ),
            (
                "statement_seconds = 86401",
                "statement_seconds must be from 2 to 86400",
            ),
            (
                "migrator_wait_seconds = 0",
                "migrator_wait_seconds must be from 1 to 300",
            ),
            (
                "migrator_wait_seconds = 301",
                "migrator_wait_seconds must be from 1 to 300",
            ),
            ("statement_seconds = -1", "statement_seconds"),
            ("statement_seconds = 1.5", "statement_seconds"),
            ("migrator_wait_seconds = true", "migrator_wait_seconds"),
            (
                "statement_seconds = 'ntnt346_private_canary'",
                "statement_seconds",
            ),
            ("statement_timeout = 5", "statement_seconds"),
        ] {
            let root = project();
            let manifest = with_limits(&root, limits);
            let d = CapturedInputs::load(&manifest, "local", None)
                .err()
                .unwrap_or_else(|| panic!("{limits} was accepted"));
            assert_eq!(d.code, "DB_CONFIG", "{limits}");
            assert!(
                d.message.contains("[databases.app.limits]") && d.message.contains(field),
                "{limits}: {}",
                d.message
            );
            assert!(!d.message.contains("canary"), "{}", d.message);
        }
        let root = project();
        let manifest = root.path().join("ntnt.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            text.replace(
                "migrations_dir=\"migrations\"\n",
                "migrations_dir=\"migrations\"\nlimits=90\n",
            ),
        )
        .unwrap();
        let d = CapturedInputs::load(&manifest, "local", None)
            .err()
            .unwrap();
        assert!(
            d.message.contains("[databases.app.limits]"),
            "{}",
            d.message
        );
    }
    #[test]
    fn catalog_digests_equal_json_digest_as_objects_change() {
        use super::super::model::Catalog;
        use serde_json::json;
        let mut digests = CatalogDigests::default();
        let mut catalog = Catalog::default();
        // An empty catalog, then objects and uncovered keys added, changed and
        // removed, with keys and values that need escaping or are not ASCII.
        assert_eq!(digests.digest(&catalog), json_digest(&catalog));
        type Step = Box<dyn Fn(&mut Catalog)>;
        let steps: Vec<Step> = vec![
            Box::new(|c| {
                c.objects
                    .insert("schema:main".into(), json!({"engine":"sqlite"}));
            }),
            Box::new(|c| {
                c.objects.insert(
                    "table:\"t \\\"q\\\"\"".into(),
                    json!({"z":1,"a":[1.5,null,true,{"y":"\u{1}\n","b":"é"}],"m":{"k":-3}}),
                );
                c.objects.insert(
                    "column:\"t\".\"x\"".into(),
                    json!([0, "x", "TEXT", 0, null, 0, 0]),
                );
            }),
            Box::new(|c| c.uncovered.push("table:\"t\"".into())),
            Box::new(|c| {
                c.objects.insert(
                    "column:\"t\".\"x\"".into(),
                    json!([0, "x", "TEXT", 1, null, 0, 0]),
                );
                c.uncovered.push("view:\"v\"".into());
            }),
            Box::new(|c| {
                c.objects.remove("schema:main");
                c.uncovered.clear();
            }),
            Box::new(|c| {
                c.objects.insert(
                    "index:\"ü\"".into(),
                    json!({"table":"t","columns":[[0,1,"x",0,"BINARY",1]]}),
                );
            }),
            Box::new(|c| c.objects.clear()),
        ];
        for step in steps {
            step(&mut catalog);
            assert_eq!(
                digests.digest(&catalog),
                json_digest(&catalog),
                "{catalog:?}"
            );
            // Digesting the same catalog again reuses every object.
            assert_eq!(digests.digest(&catalog), json_digest(&catalog));
        }
    }
    #[test]
    fn json_identity_uses_sorted_object_keys_not_rust_field_order() {
        #[derive(Serialize)]
        struct Fields {
            z: u32,
            a: u32,
        }
        assert_eq!(
            json_digest(&Fields { z: 1, a: 2 }),
            json_digest(&serde_json::json!({"a":2,"z":1}))
        );
    }
    #[test]
    fn capture_is_deterministic_numeric_and_retains_bytes_after_replacement() {
        let root = project();
        let p = root.path().join("ntnt.toml");
        std::fs::write(
            root.path().join("migrations/10_ten.sql"),
            "CREATE TABLE app.ten(id INT);",
        )
        .unwrap();
        std::fs::write(
            root.path().join("migrations/2_two.sql"),
            "CREATE TABLE app.two(id INT);",
        )
        .unwrap();
        let a = CapturedInputs::load(&p, "local", None).unwrap();
        let b = CapturedInputs::load(&p, "local", None).unwrap();
        assert_eq!(a.input_digest, b.input_digest);
        assert_eq!(
            a.migrations.iter().map(|m| m.id).collect::<Vec<_>>(),
            [2, 10]
        );
        std::fs::remove_file(root.path().join("migrations/2_two.sql")).unwrap();
        std::fs::write(
            root.path().join("migrations/2_two.sql"),
            "SELECT 'replacement';",
        )
        .unwrap();
        std::fs::write(&p, "database='replacement'").unwrap();
        assert_eq!(a.migrations[0].sql, "CREATE TABLE app.two(id INT);");
        assert_eq!(
            a.document["databases"]["app"]["schema"].as_str(),
            Some("app")
        );
    }
    #[test]
    fn metadata_presence_and_exact_comments_are_checksummed() {
        let root = project();
        let manifest = root.path().join("ntnt.toml");
        std::fs::write(
            root.path().join("migrations/1_first.sql"),
            "CREATE TABLE app.t(id INT);",
        )
        .unwrap();
        let a = CapturedInputs::load(&manifest, "local", None).unwrap();
        let side = root.path().join("migrations/1_first.toml");
        std::fs::write(&side, "").unwrap();
        let b = CapturedInputs::load(&manifest, "local", None).unwrap();
        std::fs::write(&side, "# comment only\n").unwrap();
        let c = CapturedInputs::load(&manifest, "local", None).unwrap();
        assert_ne!(a.migrations[0].checksum, b.migrations[0].checksum);
        assert_ne!(b.migrations[0].checksum, c.migrations[0].checksum);
        for metadata in [
            "requires=[]",
            "transaction='non_transactional'",
            "ensures=[]",
            "force=true",
        ] {
            std::fs::write(&side, metadata).unwrap();
            assert_eq!(
                CapturedInputs::load(&manifest, "local", None)
                    .err()
                    .unwrap()
                    .code,
                "DB_METADATA_UNSUPPORTED"
            );
        }
    }
    #[test]
    fn rejects_hardlinked_inputs_before_reading() {
        let root = project();
        std::fs::write(
            root.path().join("original.sql"),
            "CREATE TABLE app.t(id INT);",
        )
        .unwrap();
        std::fs::hard_link(
            root.path().join("original.sql"),
            root.path().join("migrations/1_first.sql"),
        )
        .unwrap();
        assert_eq!(
            CapturedInputs::load(&root.path().join("ntnt.toml"), "local", None)
                .err()
                .unwrap()
                .code,
            "DB_INPUT_IO"
        );
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_nonregular_inputs() {
        use std::os::unix::fs::symlink;
        let root = project();
        let manifest = root.path().join("ntnt.toml");
        std::fs::write(
            root.path().join("original.sql"),
            "CREATE TABLE app.t(id INT);",
        )
        .unwrap();
        let link = root.path().join("migrations/1_first.sql");
        symlink(root.path().join("original.sql"), &link).unwrap();
        assert_eq!(
            CapturedInputs::load(&manifest, "local", None)
                .err()
                .unwrap()
                .code,
            "DB_INPUT_IO"
        );
        std::fs::remove_file(&link).unwrap();
        std::fs::create_dir(&link).unwrap();
        assert_eq!(
            CapturedInputs::load(&manifest, "local", None)
                .err()
                .unwrap()
                .code,
            "DB_INPUT_IO"
        );
    }

    fn put(root: &Path, entry: &str, bytes: impl AsRef<[u8]>) {
        std::fs::write(root.join(entry), bytes).unwrap();
    }
    fn rename(root: &Path, from: &str, to: &str) {
        std::fs::rename(root.join(from), root.join(to)).unwrap();
    }
    fn remove(root: &Path, entry: &str) {
        std::fs::remove_file(root.join(entry)).unwrap();
    }
    /// A check inside the supported subset, and the one #422 reports (cli-19)
    /// with an operator outside it.
    const SUPPORTED_CHECK: &str = "SELECT NOT EXISTS (SELECT 1 FROM app.t WHERE id IS NULL)";
    const UNSUPPORTED_CHECK: &str = "SELECT NOT EXISTS (SELECT 1 FROM app.t WHERE kind = 'snmp' GROUP BY tenant_id, options->>'oid' HAVING count(*) > 1)";

    /// An entry of a project's migrations directory, beside a valid
    /// `0001_initial.sql`, that `load` refuses: how to make it, what the
    /// refusal is, and how to follow its advice.
    struct Refused {
        /// The entry as the diagnostic names it, relative to the project.
        entry: &'static str,
        code: &'static str,
        /// Fragments the message contains besides the entry.
        says: &'static [&'static str],
        make: fn(&Path),
        /// What the next action says to do.
        fix: fn(&Path),
    }
    fn refused_entries() -> Vec<Refused> {
        #[allow(unused_mut)]
        let mut cases = vec![
            Refused {
                entry: "migrations/notes.txt",
                code: "DB_CONFIG",
                says: &["only <id>_<name>.sql migrations"],
                make: |r| put(r, "migrations/notes.txt", "notes"),
                fix: |r| rename(r, "migrations/notes.txt", "notes.txt"),
            },
            Refused {
                entry: "migrations/0001_initial.sql~",
                code: "DB_CONFIG",
                says: &["only <id>_<name>.sql migrations"],
                make: |r| {
                    put(
                        r,
                        "migrations/0001_initial.sql~",
                        "CREATE TABLE app.t(id INT);",
                    )
                },
                fix: |r| rename(r, "migrations/0001_initial.sql~", "0001_initial.sql~"),
            },
            Refused {
                entry: "migrations/0002_more.SQL",
                code: "DB_CONFIG",
                says: &["only <id>_<name>.sql migrations"],
                make: |r| {
                    put(
                        r,
                        "migrations/0002_more.SQL",
                        "CREATE TABLE app.more(id INT);",
                    )
                },
                fix: |r| rename(r, "migrations/0002_more.SQL", "migrations/0002_more.sql"),
            },
            Refused {
                entry: "migrations/old",
                code: "DB_CONFIG",
                says: &["only <id>_<name>.sql migrations"],
                make: |r| std::fs::create_dir(r.join("migrations/old")).unwrap(),
                fix: |r| rename(r, "migrations/old", "old"),
            },
            Refused {
                entry: "migrations/0002_café.sql",
                code: "DB_CONFIG",
                says: &["ASCII letters"],
                make: |r| put(r, "migrations/0002_café.sql", "CREATE TABLE app.u(id INT);"),
                fix: |r| rename(r, "migrations/0002_café.sql", "migrations/0002_cafe.sql"),
            },
            Refused {
                entry: "migrations/0000_zero.sql",
                code: "DB_CONFIG",
                says: &["positive decimal integer"],
                make: |r| put(r, "migrations/0000_zero.sql", "CREATE TABLE app.z(id INT);"),
                fix: |r| rename(r, "migrations/0000_zero.sql", "migrations/0002_zero.sql"),
            },
            Refused {
                entry: "migrations/second.sql",
                code: "DB_CONFIG",
                says: &["<numeric-id>_<name>.sql"],
                make: |r| put(r, "migrations/second.sql", "CREATE TABLE app.s(id INT);"),
                fix: |r| rename(r, "migrations/second.sql", "migrations/0002_second.sql"),
            },
            Refused {
                entry: "migrations/0002_.sql",
                code: "DB_CONFIG",
                says: &["<numeric-id>_<name>.sql"],
                make: |r| put(r, "migrations/0002_.sql", "CREATE TABLE app.e(id INT);"),
                fix: |r| rename(r, "migrations/0002_.sql", "migrations/0002_e.sql"),
            },
            Refused {
                entry: "migrations/0002_old.sql",
                code: "DB_INPUT_IO",
                says: &["not a regular file"],
                make: |r| std::fs::create_dir(r.join("migrations/0002_old.sql")).unwrap(),
                fix: |r| rename(r, "migrations/0002_old.sql", "0002_old.sql"),
            },
            Refused {
                entry: "migrations/0002_hard.sql",
                code: "DB_INPUT_IO",
                says: &["more than one hard link"],
                make: |r| {
                    put(r, "original.sql", "CREATE TABLE app.h(id INT);");
                    std::fs::hard_link(r.join("original.sql"), r.join("migrations/0002_hard.sql"))
                        .unwrap();
                },
                fix: |r| {
                    remove(r, "migrations/0002_hard.sql");
                    put(r, "migrations/0002_hard.sql", "CREATE TABLE app.h(id INT);");
                },
            },
            Refused {
                entry: "migrations/01_dup.sql",
                code: "DB_DUPLICATE_ID",
                says: &["ID 1", "migrations/0001_initial.sql"],
                make: |r| put(r, "migrations/01_dup.sql", "CREATE TABLE app.d(id INT);"),
                fix: |r| rename(r, "migrations/01_dup.sql", "migrations/0002_dup.sql"),
            },
            Refused {
                entry: "migrations/0002_bytes.sql",
                code: "DB_CONFIG",
                says: &["UTF-8", "line 2, column 4"],
                make: |r| {
                    put(
                        r,
                        "migrations/0002_bytes.sql",
                        b"CREATE TABLE app.b(id INT);\n-- \xff\n",
                    )
                },
                fix: |r| {
                    put(
                        r,
                        "migrations/0002_bytes.sql",
                        "CREATE TABLE app.b(id INT);\n",
                    )
                },
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_METADATA_UNSUPPORTED",
                says: &["not valid TOML", "line 1, column 19"],
                make: |r| put(r, "migrations/0001_initial.toml", "intent_ids = [\"x\"\n"),
                fix: |r| put(r, "migrations/0001_initial.toml", "intent_ids = [\"x\"]\n"),
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_METADATA_UNSUPPORTED",
                says: &["line 2, column 1", "`force`"],
                make: |r| {
                    put(
                        r,
                        "migrations/0001_initial.toml",
                        "intent_ids = [\"x\"]\nforce = true\n",
                    )
                },
                fix: |r| put(r, "migrations/0001_initial.toml", "intent_ids = [\"x\"]\n"),
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_METADATA_UNSUPPORTED",
                says: &["line 2, column 5", "requires"],
                make: |r| put(r, "migrations/0001_initial.toml", "[requires]\nx = 1\n"),
                fix: |r| {
                    put(
                        r,
                        "migrations/0001_initial.toml",
                        format!("[requires]\nx = {SUPPORTED_CHECK:?}\n"),
                    )
                },
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_METADATA_UNSUPPORTED",
                says: &["UTF-8", "line 1, column 16"],
                make: |r| {
                    put(
                        r,
                        "migrations/0001_initial.toml",
                        b"intent_ids = [\"\xff\"]\n",
                    )
                },
                fix: |r| put(r, "migrations/0001_initial.toml", "intent_ids = [\"x\"]\n"),
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_CONFIG",
                says: &["intent_ids"],
                make: |r| put(r, "migrations/0001_initial.toml", "intent_ids = [\"\"]\n"),
                fix: |r| put(r, "migrations/0001_initial.toml", "intent_ids = [\"x\"]\n"),
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_CHECK_UNSUPPORTED",
                says: &["snmp_oid_unique", "[requires]"],
                make: |r| {
                    let sidecar = format!("[requires]\ncheck_types_known = {SUPPORTED_CHECK:?}\nsnmp_oid_unique = {UNSUPPORTED_CHECK:?}\n");
                    put(r, "migrations/0001_initial.toml", sidecar)
                },
                fix: |r| {
                    let sidecar = format!("[requires]\ncheck_types_known = {SUPPORTED_CHECK:?}\n");
                    put(r, "migrations/0001_initial.toml", sidecar)
                },
            },
            Refused {
                entry: "migrations/0001_initial.toml",
                code: "DB_METADATA_UNSUPPORTED",
                says: &["sqlite_rebuild"],
                make: |r| {
                    put(
                        r,
                        "migrations/0001_initial.toml",
                        "sqlite_rebuild = \"t\"\n",
                    )
                },
                fix: |r| put(r, "migrations/0001_initial.toml", ""),
            },
            Refused {
                entry: "migrations/0009_orphan.toml",
                code: "DB_CONFIG",
                says: &["migrations/0009_orphan.sql"],
                make: |r| put(r, "migrations/0009_orphan.toml", ""),
                fix: |r| remove(r, "migrations/0009_orphan.toml"),
            },
        ];
        #[cfg(unix)]
        {
            cases.extend([
                Refused {
                    // A control character is escaped, never written to a terminal.
                    entry: "migrations/\"0002_a\\nb.sql\"",
                    code: "DB_CONFIG",
                    says: &["ASCII letters"],
                    make: |r| put(r, "migrations/0002_a\nb.sql", "CREATE TABLE app.n(id INT);"),
                    fix: |r| remove(r, "migrations/0002_a\nb.sql"),
                },
                Refused {
                    entry: "migrations/0002_link.sql",
                    code: "DB_INPUT_IO",
                    says: &["a symbolic link"],
                    make: |r| {
                        put(r, "original.sql", "CREATE TABLE app.l(id INT);");
                        std::os::unix::fs::symlink(
                            r.join("original.sql"),
                            r.join("migrations/0002_link.sql"),
                        )
                        .unwrap();
                    },
                    fix: |r| {
                        remove(r, "migrations/0002_link.sql");
                        std::fs::copy(r.join("original.sql"), r.join("migrations/0002_link.sql"))
                            .unwrap();
                    },
                },
            ]);
        }
        // macOS file systems store only UTF-8 names, so this name cannot exist
        // there.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use std::os::unix::ffi::OsStrExt;
            cases.extend([Refused {
                // Bytes that are not UTF-8 are shown escaped.
                entry: "migrations/\"0002_\\xFF.sql\"",
                code: "DB_CONFIG",
                says: &["UTF-8"],
                make: |r| {
                    let name = std::ffi::OsStr::from_bytes(b"0002_\xff.sql");
                    std::fs::write(r.join("migrations").join(name), "SELECT 1;").unwrap()
                },
                fix: |r| {
                    let name = std::ffi::OsStr::from_bytes(b"0002_\xff.sql");
                    std::fs::remove_file(r.join("migrations").join(name)).unwrap()
                },
            }]);
        }
        cases
    }

    /// #422: every refusal of a migrations directory entry names it, in the
    /// message and in `source`, relative to the project and without the
    /// project's own path. Its next action is about the directory, not the
    /// manifest, and following it resolves the refusal.
    #[test]
    fn inventory_refusals_name_the_entry_and_advise_on_the_directory() {
        let manifest_advice = Diagnostic::new("DB_CONFIG", "x").next_action;
        let generic = Diagnostic::new("DB_NOT_A_REAL_CODE", "x").next_action;
        for case in refused_entries() {
            let root = project();
            put(
                root.path(),
                "migrations/0001_initial.sql",
                "CREATE TABLE app.t(id INT);",
            );
            (case.make)(root.path());
            let manifest = root.path().join("ntnt.toml");
            let Err(d) = CapturedInputs::load(&manifest, "local", None) else {
                panic!("{} was accepted", case.entry);
            };
            let at = format!("{}: {d} ({:?})", case.entry, d.source);
            assert_eq!(d.code, case.code, "{at}");
            assert_eq!(d.source.as_deref(), Some(case.entry), "{at}");
            assert!(d.message.starts_with(&format!("{}: ", case.entry)), "{at}");
            for fragment in case.says {
                assert!(d.message.contains(fragment), "{fragment:?} missing: {at}");
            }
            assert!(!d.message.contains(root.path().to_str().unwrap()), "{at}");
            assert_ne!(d.next_action, manifest_advice, "{at}");
            assert_ne!(d.next_action, generic, "{at}");
            assert!(
                d.next_action.contains("migrations directory") || d.next_action.contains("sidecar"),
                "{at}: {}",
                d.next_action
            );
            (case.fix)(root.path());
            if let Err(d) = CapturedInputs::load(&manifest, "local", None) {
                panic!("{} after following its advice: {d}", case.entry);
            }
        }
    }

    /// #422: names starting with `.` (`.gitkeep`, `.DS_Store`, editor swap
    /// files and lock links, macOS `._` files) and README or README.<ext> in
    /// any letter case are not part of the history. They are not read and
    /// do not change the inputs' identity. Every other entry that is not a
    /// migration or sidecar is still refused, by name.
    #[test]
    fn dotfiles_and_readmes_are_ignored() {
        let root = project();
        let manifest = root.path().join("ntnt.toml");
        put(
            root.path(),
            "migrations/0001_initial.sql",
            "CREATE TABLE app.t(id INT);",
        );
        let bare = CapturedInputs::load(&manifest, "local", None).unwrap();
        for entry in [
            ".gitkeep",
            ".DS_Store",
            ".0001_initial.sql.swp",
            "._0001_initial.sql",
            ".0002_hidden.sql",
            "README",
            "README.md",
            "readme.txt",
            "ReadMe.rst",
        ] {
            put(
                root.path(),
                &format!("migrations/{entry}"),
                "not a migration",
            );
        }
        std::fs::create_dir(root.path().join("migrations/.hidden")).unwrap();
        std::fs::create_dir(root.path().join("migrations/README.d")).unwrap();
        #[cfg(unix)]
        {
            // An Emacs lock link points nowhere; it is never followed.
            std::os::unix::fs::symlink(
                "user@host.1:1",
                root.path().join("migrations/.#0001_initial.sql"),
            )
            .unwrap();
        }
        // macOS file systems store only UTF-8 names.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::OsStr::from_bytes(b".\xff");
            std::fs::write(root.path().join("migrations").join(name), "x").unwrap();
        }
        let ignoring = CapturedInputs::load(&manifest, "local", None).unwrap();
        assert_eq!(ignoring.input_digest, bare.input_digest);
        assert_eq!(ignoring.migrations.len(), 1);
        for entry in [
            "README.md.bak",
            "README_.md",
            "READMEFILE",
            "readme.",
            "notes.txt",
            "0002_more.SQL",
            "0001_initial.sql~",
        ] {
            // Windows drops a trailing dot, so `readme.` would be created as
            // the README `readme`.
            if cfg!(windows) && entry.ends_with('.') {
                continue;
            }
            put(root.path(), &format!("migrations/{entry}"), "x");
            let d = CapturedInputs::load(&manifest, "local", None)
                .err()
                .unwrap_or_else(|| panic!("{entry} was accepted"));
            assert_eq!(d.source, Some(format!("migrations/{entry}")), "{d}");
            remove(root.path(), &format!("migrations/{entry}"));
        }
    }

    /// #422: a file that changes between the checks before and after it is
    /// read is reported as changed, naming it as the user gave it.
    #[test]
    fn a_file_that_changes_while_it_is_captured_is_named() {
        fn grow_then_open(path: &Path) -> std::io::Result<File> {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(path)?
                .write_all(b"\n# appended while ntnt db read the file\n")?;
            super::super::input_file::open(path)
        }
        let root = project();
        let manifest = root.path().join("ntnt.toml");
        let d = capture(Input::Manifest(&manifest), grow_then_open, |_| Ok(()))
            .err()
            .unwrap();
        assert_eq!(d.code, "DB_INPUT_IO", "{d}");
        assert!(
            d.message.contains("ntnt.toml")
                && d.message.contains("changed while it was being read"),
            "{d}"
        );
        assert!(d.next_action.contains("Rerun"), "{d}");
    }

    /// Sidecar refusals give a syntax error's position as toml's own message
    /// does, at the end of the input too.
    #[test]
    fn sidecar_positions_are_the_ones_toml_reports() {
        for text in [
            "intent_ids = [\"x\"\n",
            "intent_ids = [\"x\"",
            "a = 1\nb = \n",
            "[requires\nx = 'y'",
            "é = [\n",
            "x = \"é\nz\"",
            "\n\n[requires]\nok = 'x'\n[requires]\n",
        ] {
            let error = text.parse::<toml::Table>().unwrap_err();
            let position = toml_position(text, error.span().unwrap().start);
            assert!(
                error
                    .to_string()
                    .starts_with(&format!("TOML parse error at {position}\n")),
                "{text:?}: {position} / {error}"
            );
        }
    }

    /// #422: an unsupported check is named with its phase, and only the
    /// checks outside the subset are named.
    #[test]
    fn unsupported_checks_are_named_with_their_phase() {
        let root = project();
        put(
            root.path(),
            "migrations/0001_initial.sql",
            "CREATE TABLE app.t(id INT);",
        );
        put(
            root.path(),
            "migrations/0001_initial.toml",
            format!("[requires]\nok = {SUPPORTED_CHECK:?}\nfirst = {UNSUPPORTED_CHECK:?}\n[ensures]\nsecond = \"SELECT 1 FROM app.t; DELETE FROM app.t\"\n\"bad name\" = {SUPPORTED_CHECK:?}\n"),
        );
        let d = CapturedInputs::load(&root.path().join("ntnt.toml"), "local", None)
            .err()
            .unwrap();
        assert_eq!(d.code, "DB_CHECK_UNSUPPORTED", "{d}");
        for fragment in [
            "first ([requires])",
            "second ([ensures])",
            "\"bad name\" ([ensures])",
        ] {
            assert!(d.message.contains(fragment), "{fragment:?} missing: {d}");
        }
        assert!(!d.message.contains("ok ("), "{d}");
        assert!(!d.message.contains("DELETE"), "{d}");
    }
}
