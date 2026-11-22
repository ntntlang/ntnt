use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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
pub(crate) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && s.as_bytes()[0].is_ascii_alphabetic()
}
/// Capture one bounded regular file and reject links or detected replacement/mutation.
/// Callers retain these bytes instead of reopening a validated path during execution.
pub(super) fn file_bytes(path: &Path) -> Result<Vec<u8>> {
    Ok(capture(path, super::input_file::open, |_| Ok(()))?.1)
}
/// `file_bytes` through `open`, running `before_read` on the opened handle (`ntnt db
/// init` locks the manifest there) and returning the handle with the bytes.
pub(super) fn capture(
    path: &Path,
    open: fn(&Path) -> std::io::Result<File>,
    before_read: impl FnOnce(&File) -> Result<()>,
) -> Result<(File, Vec<u8>)> {
    let bad = || {
        Diagnostic::new(
            "DB_INPUT_IO",
            "Cannot capture a regular, unlinked input file (maximum 1 MiB).",
        )
    };
    let meta = std::fs::symlink_metadata(path).map_err(|_| bad())?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_INPUT_BYTES as u64 {
        return Err(bad());
    }
    let file = open(path).map_err(|_| bad())?;
    before_read(&file)?;
    let opened = file.metadata().map_err(|_| bad())?;
    if !opened.is_file()
        || opened.file_type().is_symlink()
        || !super::input_file::single_link(&file, &opened)
    {
        return Err(bad());
    }
    let mut bytes = Vec::new();
    (&file)
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| bad())?;
    let after = std::fs::symlink_metadata(path).map_err(|_| bad())?;
    let finished = file.metadata().map_err(|_| bad())?;
    if bytes.len() > MAX_INPUT_BYTES
        || !super::input_file::same(&meta, &opened)
        || !super::input_file::same(&opened, &after)
        || !super::input_file::same(&opened, &finished)
        || !super::input_file::single_link(&file, &finished)
    {
        return Err(bad());
    }
    Ok((file, bytes))
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
            checksum: json_digest(
                &serde_json::json!({"format":"ntnt-baseline-v1","prefix":prefix}),
            ),
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
        let database_name = match database {
            Some(name) if databases.contains_key(name) => name.to_owned(),
            Some(_) => return Err(Diagnostic::new("DB_DATABASE_UNKNOWN", "The selected logical database is not declared; no fallback database was selected.")),
            None if databases.len() == 1 => databases.keys().next().unwrap().clone(),
            None => return Err(Diagnostic::new("DB_DATABASE_REQUIRED", "Multiple logical databases are declared; select one with --database before choosing its target.")),
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
        match config.engine.as_str() {
            "postgresql"
                if identifier(&config.schema)
                    && !["information_schema", "_ntnt"]
                        .contains(&config.schema.as_str())
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
        if config.adoption_checks.len() > 64
            || config.adoption_checks.iter().any(|(name, sql)| {
                !identifier(name)
                    || sql.len() > 65536
                    || !super::analysis::contract_supported(sql, &config.schema)
            })
        {
            return Err(Diagnostic::new("DB_CHECK_UNSUPPORTED", "Adoption checks require bounded names and the supported schema-qualified SELECT expression subset."));
        }
        Ok((database_name, config))
    }
    /// Resolve the selected target and validate its endpoint, scratch and approval
    /// references. Secret names are validated here; their values are never read.
    pub(super) fn endpoint(config: &Database, target: &str) -> Result<Endpoint> {
        let endpoint = config
            .targets
            .get(target)
            .cloned()
            .ok_or_else(|| invalid("Select a declared database target."))?;
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
        let bytes = file_bytes(manifest)?;
        let document = Self::parse(&bytes)?;
        let (database_name, config) = Self::select(&document, database)?;
        let endpoint = Self::endpoint(&config, target)?;
        let dir = Self::migrations_dir(manifest, &config)?;
        let mut sources = BTreeMap::new();
        let mut total_bytes = 0usize;
        for entry in
            std::fs::read_dir(&dir).map_err(|_| invalid("Cannot enumerate migration directory."))?
        {
            let entry = entry.map_err(|_| invalid("Cannot enumerate migration directory."))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("Migration filenames must be UTF-8."))?;
            if !(name.ends_with(".sql") || name.ends_with(".toml")) {
                return Err(invalid(
                    "Migration directory accepts only SQL and matching TOML sidecars.",
                ));
            }
            if sources.len() >= 2048 {
                return Err(invalid(
                    "Too many migration files (maximum 1024 SQL/sidecar pairs).",
                ));
            }
            let bytes = file_bytes(&entry.path())?;
            total_bytes += bytes.len();
            if total_bytes > 16 * 1_048_576 {
                return Err(invalid("Migration inventory exceeds 16 MiB."));
            }
            sources.insert(name, bytes);
        }
        let mut ids = BTreeSet::new();
        let mut migrations = Vec::new();
        for (name, sql_bytes) in sources.iter().filter(|(name, _)| name.ends_with(".sql")) {
            let stem = name.strip_suffix(".sql").unwrap();
            let (number, description) = stem
                .split_once('_')
                .ok_or_else(|| invalid("Use <numeric-id>_<name>.sql filenames."))?;
            let id = number
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0 && number.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| invalid("Migration ID must be a positive decimal integer."))?;
            if description.is_empty()
                || !description
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(invalid(
                    "Migration names use ASCII letters, numbers, underscores or hyphens.",
                ));
            }
            if !ids.insert(id) {
                return Err(Diagnostic::new(
                    "DB_DUPLICATE_ID",
                    "Duplicate numeric migration ID.",
                ));
            }
            let sql =
                String::from_utf8(sql_bytes.clone()).map_err(|_| invalid("SQL must be UTF-8."))?;
            let metadata = sources.get(&format!("{stem}.toml")).cloned();
            let parsed: Metadata = match &metadata {
                Some(b) => std::str::from_utf8(b).ok().and_then(|s| toml::from_str(s).ok())
                    .ok_or_else(|| Diagnostic::new("DB_METADATA_UNSUPPORTED", "Supported metadata: intent_ids and named requires/ensures SQL maps; execution modes are not supported."))?,
                None => Metadata::default(),
            };
            if parsed
                .intent_ids
                .iter()
                .any(|s| s.len() > 128 || s.is_empty() || s.chars().any(char::is_control))
            {
                return Err(invalid("Invalid Intent traceability ID."));
            }
            if parsed.requires.len() + parsed.ensures.len() > 64
                || parsed
                    .requires
                    .iter()
                    .chain(&parsed.ensures)
                    .any(|(name, sql)| {
                        !identifier(name)
                            || sql.len() > 65536
                            || !super::analysis::contract_supported(sql, &config.schema)
                    })
            {
                return Err(Diagnostic::new("DB_CHECK_UNSUPPORTED", "Checks require bounded names and the documented closed SELECT expression subset; unsupported SQL is never executed."));
            }
            if parsed.sqlite_rebuild.as_ref().is_some_and(|table| {
                config.engine != "sqlite"
                    || !identifier(table)
                    || table.to_ascii_lowercase().starts_with("sqlite_")
            }) {
                return Err(Diagnostic::new(
                    "DB_METADATA_UNSUPPORTED",
                    "sqlite_rebuild requires one ordinary table name and the SQLite adapter.",
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
            if !sources.contains_key(&format!("{}.sql", name.strip_suffix(".toml").unwrap())) {
                return Err(invalid("Orphan migration sidecar."));
            }
        }
        if migrations.len() > 1024 {
            return Err(invalid("Migration inventory exceeds 1024 migrations."));
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
}
