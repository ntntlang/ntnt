use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Endpoint {
    pub connection_secret: String,
    pub database: String,
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
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    #[serde(default)]
    intent_ids: Vec<String>,
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
}
impl Migration {
    /// Expose source identities and traceability without raw SQL or sidecar contents.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id.to_string(), "source":self.name,
            "checksum":self.checksum, "sql_digest":digest(&[self.sql.as_bytes()]), "metadata_digest":self.metadata.as_ref().map(|b| digest(&[b])), "intent_ids":self.intent_ids})
    }
}

pub(crate) struct CapturedInputs {
    pub config: Database,
    pub database_name: String,
    pub target_name: String,
    pub endpoint: Endpoint,
    pub document: toml::Value,
    pub migrations: Vec<Migration>,
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
    let file = super::input_file::open(path).map_err(|_| bad())?;
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
    Ok(bytes)
}
impl CapturedInputs {
    /// Capture the manifest and exactly one logical database's ordered migration inputs.
    /// Ambiguous selection and malformed registry entries fail before credential lookup;
    /// unselected stores' directories are not visited. Exact bytes and logical selection
    /// participate in plan identity, while secret resolution remains a later operation.
    pub fn load(manifest: &Path, target: &str, database: Option<&str>) -> Result<Self> {
        let bytes = file_bytes(manifest)?;
        let document: toml::Value = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| invalid("Invalid UTF-8/TOML manifest."))?;
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
        let config: Database = databases[&database_name]
            .clone()
            .try_into()
            .map_err(|_| invalid("Unknown or invalid database configuration field."))?;
        if config.engine != "postgresql"
            || !identifier(&config.schema)
            || config.schema == "public"
            || config.schema == "information_schema"
            || config.schema == "_ntnt"
            || config.schema.starts_with("pg_")
        {
            return Err(invalid(
                "Use PostgreSQL and an explicit non-system managed schema (not public).",
            ));
        }
        let endpoint = config
            .targets
            .get(target)
            .cloned()
            .ok_or_else(|| invalid("Select a declared database target."))?;
        for ep in std::iter::once(&endpoint).chain(config.scratch.iter().map(|s| &s.endpoint)) {
            if !identifier(&ep.database)
                || !identifier(&ep.role)
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
                    .ok_or_else(|| Diagnostic::new("DB_METADATA_UNSUPPORTED", "Only intent_ids traceability metadata is supported; checks and execution modes are not implemented."))?,
                None => Metadata::default(),
            };
            if parsed
                .intent_ids
                .iter()
                .any(|s| s.len() > 128 || s.is_empty() || s.chars().any(char::is_control))
            {
                return Err(invalid("Invalid Intent traceability ID."));
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
            &serde_json::json!({"policy":"read-only-target/restricted-replay-v1", "database_name":database_name, "database":config}),
        );
        let input_digest = digest(&[
            b"ntnt-inputs-v1",
            &bytes,
            database_name.as_bytes(),
            json_digest(&summaries).as_bytes(),
        ]);
        Ok(Self {
            config,
            database_name,
            target_name: target.into(),
            endpoint,
            document,
            migrations,
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
