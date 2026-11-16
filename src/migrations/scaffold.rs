//! First-time setup: `ntnt db init` declares one logical database and its secret
//! names in ntnt.toml; `ntnt db new` creates the next numbered SQL migration.
//! Neither command reads a secret value, connects to a database or replaces a file.
use super::inventory::{self, CapturedInputs};
use super::{Diagnostic, Engine, InitArgs, NewArgs, Result};
use serde_json::{json, Value};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// A scaffolded database keeps its migrations in this directory beside the manifest.
const MIGRATIONS_DIR: &str = "migrations";

fn conflict(message: &str) -> Diagnostic {
    Diagnostic::new("DB_INIT_CONFLICT", message)
}
fn io_failure(message: &str) -> Diagnostic {
    Diagnostic::new("DB_SCAFFOLD_IO", message)
}
/// The directory that owns the manifest's relative migrations directory.
fn project_dir(manifest: &Path) -> Option<&Path> {
    manifest.parent().filter(|p| !p.as_os_str().is_empty())
}

/// Init names become secret prefixes and PostgreSQL schema, role and database names:
/// lowercase identifiers that fit 63 bytes with the longest derived suffix
/// (`_replay_owner`) and avoid reserved PostgreSQL names.
fn valid_database_name(name: &str) -> bool {
    name.len() <= 50
        && name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !name.starts_with("pg_")
        && name != "information_schema"
}
/// Migration names become the `<id>_<name>.sql` filename suffix.
fn valid_migration_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The commented manifest section for one database plus its declared secret names
/// (target, scratch, approval). It mirrors docs/DATABASE_MIGRATIONS.md#quick-start
/// and contains names only, never a path, connection string or digest.
fn section(engine: Engine, name: &str) -> (String, [String; 3]) {
    let prefix = name.to_ascii_uppercase();
    let approval = format!("{prefix}_MIGRATION_APPROVAL");
    match engine {
        Engine::Sqlite => {
            let target = format!("{prefix}_DATABASE_FILE");
            let replay = format!("{prefix}_REPLAY_FILE");
            let text = format!(
                r#"# Logical database "{name}" (SQLite), declared by `ntnt db init`.
# Secret values come from the std/secrets provider. Never write file paths,
# credentials or approval digests in this file.
# Guide: docs/DATABASE_MIGRATIONS.md#quick-start
[databases.{name}]
engine = "sqlite"
schema = "main"
migrations_dir = "{MIGRATIONS_DIR}"
# migrate runs a plan only when this secret equals its reviewed plan_digest.
approval_secret = "{approval}"

# {target}: absolute path of an existing SQLite file (an empty file is valid).
[databases.{name}.targets.development]
connection_secret = "{target}"

# {replay}: absolute path of a separate, empty, disposable SQLite file.
[databases.{name}.scratch]
connection_secret = "{replay}"
disposable = true

# Secret declarations: names only.
[secrets.{target}]
[secrets.{replay}]
[secrets.{approval}]
"#
            );
            (text, [target, replay, approval])
        }
        Engine::Postgresql => {
            let target = format!("{prefix}_DATABASE");
            let replay = format!("{prefix}_REPLAY_DATABASE");
            let text = format!(
                r#"# Logical database "{name}" (PostgreSQL), declared by `ntnt db init`.
# Secret values come from the std/secrets provider. Never write connection
# strings, credentials or approval digests in this file.
# Guide: docs/DATABASE_MIGRATIONS.md#quick-start
[databases.{name}]
engine = "postgresql"
# The managed schema must already exist in the target database.
schema = "{name}"
migrations_dir = "{MIGRATIONS_DIR}"
# migrate runs a plan only when this secret equals its reviewed plan_digest.
approval_secret = "{approval}"

# {target}: libpq key/value connection string for this database and role.
[databases.{name}.targets.development]
connection_secret = "{target}"
database = "{name}_development"
role = "{name}_owner"

# {replay}: a separate, empty, disposable database with its own owner role.
[databases.{name}.scratch]
connection_secret = "{replay}"
database = "{name}_replay"
role = "{name}_replay_owner"
disposable = true

# Secret declarations: names only.
[secrets.{target}]
[secrets.{replay}]
[secrets.{approval}]
"#
            );
            (text, [target, replay, approval])
        }
    }
}

/// Local-development steps after init, consistent with the documented quick start.
fn next_steps(engine: Engine, name: &str, secrets: &[String; 3], flags: &str) -> Vec<String> {
    let [target, replay, approval] = secrets;
    let (provision, values) = match engine {
        Engine::Sqlite => (
            "Create an empty development SQLite file and a separate empty scratch file; ntnt db never creates database files.".to_string(),
            "the absolute paths of those files",
        ),
        Engine::Postgresql => (
            format!(
                "Provision database {name}_development owned by login role {name}_owner{schema}; provision a separate empty database {name}_replay owned by login role {name}_replay_owner.",
                schema = if name == "public" {
                    String::new()
                } else {
                    format!(" and create schema {name} in it as that role")
                }
            ),
            "libpq key/value strings with one host, the role as user, the database as dbname and sslmode=disable",
        ),
    };
    vec![
        provision,
        format!("For local development, export NTNT_ENV=development NTNT_SECRETS_PROVIDER=env and set {target} and {replay} to {values}."),
        format!("Create the first migration with `ntnt db new initial{flags}`, then write its SQL."),
        format!("Run `ntnt db status --target development{flags}`, then `ntnt db plan --target development{flags} --out review.json`."),
        format!("Review review.json and the SQL, set {approval} to its plan_digest, then run `ntnt db migrate --target development{flags} --plan review.json`."),
    ]
}

/// Refuse to change existing declarations or to share another database's migrations.
fn refuse_existing(document: &toml::Value, name: &str, secrets: &[String; 3]) -> Result<()> {
    let table = |key: &str| document.get(key).and_then(toml::Value::as_table);
    if table("databases").is_some_and(|t| t.contains_key(name)) {
        return Err(conflict(&format!(
            "The manifest already declares [databases.{name}]."
        )));
    }
    if let Some(secret) = secrets
        .iter()
        .find(|s| table("secrets").is_some_and(|t| t.contains_key(s.as_str())))
    {
        return Err(conflict(&format!(
            "The manifest already declares [secrets.{secret}]."
        )));
    }
    // A database would otherwise read another's files (equal or nested directory).
    let shared = |dir: &str| {
        dir.split('/')
            .find(|part| !part.is_empty() && *part != ".")
            .is_some_and(|first| first.eq_ignore_ascii_case(MIGRATIONS_DIR))
    };
    if table("databases")
        .into_iter()
        .flatten()
        .any(|(_, database)| {
            database
                .get("migrations_dir")
                .and_then(toml::Value::as_str)
                .is_some_and(shared)
        })
    {
        return Err(conflict(
            "Another declared database already uses the migrations directory; each database needs its own.",
        ));
    }
    Ok(())
}

/// Create a file that must not exist yet. `create_new` never truncates or replaces a
/// file, including one created concurrently. A partially written new file is removed.
fn create_new(path: &Path, contents: &[u8], exists: Diagnostic, label: &str) -> Result<()> {
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Err(exists),
        Err(_) => return Err(io_failure(&format!("Cannot create {label}."))),
    };
    if file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(io_failure(&format!(
            "Cannot write {label}; the incomplete new file was removed."
        )));
    }
    Ok(())
}

/// Replace an existing manifest with `combined` (its exact captured bytes plus the
/// new section) by renaming a synced sibling file over it, so a failure leaves the
/// original intact. A concurrent edit or a manifest the caller cannot write aborts.
fn replace_manifest(path: &Path, original: &[u8], combined: &[u8]) -> Result<()> {
    let failed =
        || io_failure("Cannot write the extended manifest; the original was left unchanged.");
    // Opening for append writes nothing; it refuses a file the caller may not modify.
    let mut probe = std::fs::OpenOptions::new();
    probe.append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        probe.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let permissions = probe
        .open(path)
        .and_then(|file| file.metadata())
        .map_err(|_| io_failure("The manifest is not writable; it was left unchanged."))?
        .permissions();
    let mut temp = tempfile::Builder::new()
        .prefix(".ntnt.toml.")
        .suffix(".tmp")
        .tempfile_in(project_dir(path).unwrap_or(Path::new(".")))
        .map_err(|_| failed())?;
    temp.write_all(combined)
        .and_then(|()| temp.as_file().set_permissions(permissions))
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|_| failed())?;
    // Replace only the exact bytes that were validated; never lose a concurrent edit.
    if inventory::file_bytes(path)? != original {
        return Err(conflict(
            "The manifest changed while init was running; it was left unchanged.",
        ));
    }
    temp.persist(path).map_err(|_| failed())?;
    Ok(())
}

/// Render a scaffolding result like the other db commands: a versioned JSON report
/// with --json, otherwise concise text. Any failure exits nonzero.
fn finish(
    command: &str,
    json_output: bool,
    result: Result<(Value, Vec<String>)>,
) -> anyhow::Result<()> {
    let passed = result.is_ok();
    let (mut report, lines) = result.unwrap_or_else(|d| {
        let line = format!("{}: {}\n  Next: {}", d.code, d.message, d.next_action);
        (json!({ "diagnostics": [d] }), vec![line])
    });
    report["format_version"] = json!(1);
    report["command"] = json!(command);
    report["outcome"] = json!(if passed { "pass" } else { "fail" });
    if report.get("diagnostics").is_none() {
        report["diagnostics"] = json!([]);
    }
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Database {command}: {}",
            report["outcome"].as_str().unwrap_or("fail")
        );
        for line in lines {
            println!("{line}");
        }
    }
    if passed {
        Ok(())
    } else {
        anyhow::bail!("Database {command} did not pass")
    }
}

/// `ntnt db init`: declare one logical database (secret names only) and create its
/// migrations directory. An existing manifest keeps its exact bytes as a prefix and
/// existing declarations, files and directories are never changed.
pub(super) fn init(args: &InitArgs) -> anyhow::Result<()> {
    finish("init", args.json, initialize(args))
}

fn initialize(args: &InitArgs) -> Result<(Value, Vec<String>)> {
    let name = args.database.as_str();
    if !valid_database_name(name) {
        return Err(Diagnostic::new(
            "DB_INIT_NAME",
            "The --database name cannot derive valid secret, role and database names.",
        ));
    }
    let engine = match args.engine {
        Engine::Sqlite => "sqlite",
        Engine::Postgresql => "postgresql",
    };
    let (section, secrets) = section(args.engine, name);
    let manifest = args.manifest.as_path();
    // An existing manifest is captured under the same file rules as every db command.
    let original = match std::fs::symlink_metadata(manifest) {
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        _ => Some(inventory::file_bytes(manifest)?),
    };
    let mut combined = Vec::new();
    if let Some(bytes) = &original {
        refuse_existing(&CapturedInputs::parse(bytes)?, name, &secrets)?;
        combined.extend_from_slice(bytes);
        if !bytes.is_empty() {
            if !bytes.ends_with(b"\n") {
                combined.push(b'\n');
            }
            combined.push(b'\n');
        }
    }
    combined.extend_from_slice(section.as_bytes());
    // Validate the complete manifest as `ntnt db` reads it before writing anything.
    if combined.len() > inventory::MAX_INPUT_BYTES {
        return Err(conflict(
            "The extended manifest would exceed the 1 MiB manifest limit.",
        ));
    }
    let document: toml::Value = std::str::from_utf8(&combined)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            conflict("The manifest cannot be extended with [databases] and [secrets] tables; it may define them inline.")
        })?;
    let (_, config) = CapturedInputs::select(&document, Some(name))?;
    CapturedInputs::endpoint(&config, "development")?;
    let migrations = project_dir(manifest).map_or_else(
        || PathBuf::from(MIGRATIONS_DIR),
        |dir| dir.join(MIGRATIONS_DIR),
    );
    let create_dir = match std::fs::symlink_metadata(&migrations) {
        Ok(metadata) if metadata.is_dir() => false,
        Ok(_) => {
            return Err(conflict(
                "The migrations path exists but is not a directory; init never replaces it.",
            ))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => true,
        Err(_) => return Err(io_failure("Cannot inspect the migrations directory.")),
    };
    if create_dir {
        std::fs::create_dir(&migrations).map_err(|_| {
            io_failure("Cannot create the migrations directory; the manifest was not changed.")
        })?;
    }
    let written = match &original {
        None => create_new(
            manifest,
            &combined,
            conflict("The manifest was created by another process while init was running; it was left unchanged."),
            "the manifest",
        ),
        Some(bytes) => replace_manifest(manifest, bytes, &combined),
    };
    if let Err(diagnostic) = written {
        if create_dir {
            // remove_dir removes only an empty directory; a concurrently used one stays.
            let _ = std::fs::remove_dir(&migrations);
        }
        return Err(diagnostic);
    }

    let declared = document
        .get("databases")
        .and_then(toml::Value::as_table)
        .map_or(0, |databases| databases.len());
    let mut flags = String::new();
    if manifest != Path::new("ntnt.toml") {
        flags.push_str(&format!(" --manifest {}", manifest.display()));
    }
    if declared > 1 {
        flags.push_str(&format!(" --database {name}"));
    }
    let steps = next_steps(args.engine, name, &secrets, &flags);
    let mut lines = vec![
        if original.is_some() {
            format!(
                "Appended [databases.{name}] ({engine}) to {}.",
                manifest.display()
            )
        } else {
            format!(
                "Created {} with [databases.{name}] ({engine}).",
                manifest.display()
            )
        },
        format!(
            "{} {}{}.",
            if create_dir {
                "Created"
            } else {
                "Using existing"
            },
            migrations.display(),
            std::path::MAIN_SEPARATOR
        ),
        format!(
            "Declared secret names (values are never written): {}.",
            secrets.join(", ")
        ),
        "Next steps:".to_string(),
    ];
    lines.extend(
        steps
            .iter()
            .enumerate()
            .map(|(index, step)| format!("  {}. {step}", index + 1)),
    );
    lines.push("See docs/DATABASE_MIGRATIONS.md#quick-start.".to_string());
    let report = json!({
        "database": name,
        "engine": engine,
        "manifest": {
            "path": manifest.display().to_string(),
            "action": if original.is_some() { "appended" } else { "created" },
        },
        "migrations_dir": {
            "path": migrations.display().to_string(),
            "action": if create_dir { "created" } else { "existing" },
        },
        "secrets": secrets,
        "next_steps": steps,
    });
    Ok((report, lines))
}

/// `ntnt db new`: create the next numbered SQL migration for the selected logical
/// database. It reads only the manifest and the migration directory listing.
pub(super) fn new(args: &NewArgs) -> anyhow::Result<()> {
    finish("new", args.json, create_migration(args))
}

/// The ID after the highest `<digits>_<name>.sql|.toml` entry, and the widest existing
/// ID (at least four digits) so a new file keeps the directory's zero padding.
fn next_id(dir: &Path) -> Result<(u64, usize)> {
    let unreadable = || Diagnostic::new("DB_CONFIG", "Cannot enumerate migration directory.");
    let (mut highest, mut width) = (0u64, 4usize);
    for entry in std::fs::read_dir(dir).map_err(|_| unreadable())? {
        let name = entry.map_err(|_| unreadable())?.file_name();
        let Some((digits, _)) = name
            .to_str()
            .and_then(|n| n.strip_suffix(".sql").or_else(|| n.strip_suffix(".toml")))
            .and_then(|stem| stem.split_once('_'))
            .filter(|(digits, _)| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let id: u64 = digits.parse().map_err(|_| {
            Diagnostic::new(
                "DB_CONFIG",
                "Migration ID must be a positive decimal integer.",
            )
        })?;
        highest = highest.max(id);
        width = width.max(digits.len());
    }
    let next = highest.checked_add(1).ok_or_else(|| {
        Diagnostic::new(
            "DB_CONFIG",
            "No migration ID follows the highest existing ID.",
        )
    })?;
    Ok((next, width))
}

fn create_migration(args: &NewArgs) -> Result<(Value, Vec<String>)> {
    if !valid_migration_name(&args.name) {
        return Err(Diagnostic::new(
            "DB_MIGRATION_NAME",
            "Invalid migration name.",
        ));
    }
    let bytes = inventory::file_bytes(&args.manifest)?;
    let document = CapturedInputs::parse(&bytes)?;
    let (database, config) = CapturedInputs::select(&document, args.database.as_deref())?;
    let dir = CapturedInputs::migrations_dir(&args.manifest, &config)?;
    let (id, width) = next_id(&dir)?;
    let source = format!("{id:0width$}_{}.sql", args.name);
    let header = format!(
        "-- {source}: database \"{database}\" ({engine}).\n\
         -- Write the SQL below. Once this migration has been applied anywhere,\n\
         -- never edit or renumber it; add a new migration instead.\n",
        engine = config.engine
    );
    create_new(
        &dir.join(&source),
        header.as_bytes(),
        Diagnostic::new(
            "DB_MIGRATION_EXISTS",
            "The next migration file already exists.",
        ),
        "the migration file",
    )?;
    let mut shown = project_dir(&args.manifest)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    shown.extend(config.migrations_dir.split('/'));
    shown.push(&source);
    let lines = vec![
        format!(
            "Created {} for database {database} ({}).",
            shown.display(),
            config.engine
        ),
        "Write its SQL, then review it with `ntnt db plan --target <target> --out review.json`."
            .to_string(),
    ];
    let report = json!({
        "database": database,
        "engine": config.engine,
        "migration": {
            "id": id.to_string(),
            "source": source,
            "path": shown.display().to_string(),
        },
    });
    Ok((report, lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_files_never_replace_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0001_initial.sql");
        std::fs::write(&path, "CREATE TABLE kept (id INTEGER);\n").unwrap();
        let error = create_new(
            &path,
            b"-- replacement\n",
            Diagnostic::new("DB_MIGRATION_EXISTS", "exists"),
            "the migration file",
        )
        .unwrap_err();
        assert_eq!(error.code, "DB_MIGRATION_EXISTS");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "CREATE TABLE kept (id INTEGER);\n"
        );
    }

    #[test]
    fn manifest_replacement_never_loses_a_concurrent_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        // init validated "[lint]\n"; the file now holds a different edit.
        std::fs::write(&path, "[lint]\nstrict = true\n").unwrap();
        let error = replace_manifest(&path, b"[lint]\n", b"[lint]\n\n[secrets.APP_DATABASE]\n")
            .unwrap_err();
        assert_eq!(error.code, "DB_INIT_CONFLICT");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[lint]\nstrict = true\n"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "the temporary file is removed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn manifest_replacement_keeps_the_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, "[lint]\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        replace_manifest(&path, b"[lint]\n", b"[lint]\n\n[secrets.APP_DATABASE]\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[lint]\n\n[secrets.APP_DATABASE]\n"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn generated_sections_parse_and_declare_only_names() {
        for engine in [Engine::Sqlite, Engine::Postgresql] {
            let (text, secrets) = section(engine, "app");
            let document = CapturedInputs::parse(text.as_bytes()).unwrap();
            let (name, config) = CapturedInputs::select(&document, None).unwrap();
            assert_eq!(name, "app");
            let endpoint = CapturedInputs::endpoint(&config, "development").unwrap();
            assert_eq!(endpoint.connection_secret, secrets[0]);
            assert_eq!(config.approval_secret.as_deref(), Some(secrets[2].as_str()));
            for secret in &secrets {
                assert!(document["secrets"][secret.as_str()]
                    .as_table()
                    .unwrap()
                    .is_empty());
            }
        }
    }
}
