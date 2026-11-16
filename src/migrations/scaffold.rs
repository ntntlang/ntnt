//! First-time setup: `ntnt db init` declares one logical database and its secret
//! names in ntnt.toml; `ntnt db new` creates the next numbered SQL migration.
//! Neither command reads a secret value, connects to a database or replaces a file.
use super::input_file;
use super::inventory::{self, CapturedInputs};
use super::{Diagnostic, Engine, InitArgs, NewArgs, Result};
use serde_json::{json, Value};
use std::fs::{File, TryLockError};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
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

/// `value` as one POSIX shell word: unchanged when no character is special to a
/// shell, otherwise single-quoted with each `'` written as `'\''`.
fn shell_word(value: &str) -> String {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b"_./:@%+=,-".contains(&b);
    if !value.is_empty() && value.bytes().all(plain) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// The arguments that keep a printed `ntnt db` command on the same manifest and
/// logical database, as shell words.
fn selection_flags(manifest: &Path, database: Option<&str>) -> String {
    let mut flags = String::new();
    if manifest != Path::new("ntnt.toml") {
        flags += &format!(" --manifest {}", shell_word(&manifest.to_string_lossy()));
    }
    if let Some(database) = database {
        flags += &format!(" --database {}", shell_word(database));
    }
    flags
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

/// Another writer replaced or edited the manifest after init captured it.
const CHANGED: &str = "The manifest changed while init was running; it was left unchanged.";

/// Whether `path` still names the open `file`, unchanged (on Unix the same device and
/// inode; on Windows the same type, size and modification time).
fn names(path: &Path, file: &File) -> bool {
    match (file.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(held), Ok(current)) => input_file::same(&held, &current),
        _ => false,
    }
}

/// How init attempts the manifest lock: `File::try_lock`, replaced in tests.
type TryLock = fn(&File) -> std::result::Result<(), TryLockError>;

/// Take init's exclusive lock on `file`, the manifest at `path`, without waiting.
/// Every init holds it from capturing the manifest until its write is complete, so
/// a concurrent init refuses instead of replacing the manifest with stale bytes.
fn lock(path: &Path, file: &File, try_lock: TryLock) -> Result<()> {
    let locked = try_lock(file);
    // A lock on a file that was replaced meanwhile protects nothing.
    if !names(path, file) {
        return Err(conflict(CHANGED));
    }
    match locked {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(conflict(
            "Another `ntnt db init` is updating the manifest; it was left unchanged.",
        )),
        Err(TryLockError::Error(_)) => Err(io_failure(
            "Cannot lock the manifest for an exclusive update; it was left unchanged.",
        )),
    }
}

/// The locked file's current bytes, read through its own handle: on Windows the
/// lock also denies reads through every other handle.
fn reread(mut file: &File) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.take(inventory::MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// Create a file that must not exist yet, open for writing. `create_new` never
/// truncates or replaces a file, including one created concurrently.
fn create_new(path: &Path, exists: Diagnostic, label: &str) -> Result<File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| match error.kind() {
            ErrorKind::AlreadyExists => exists,
            _ => io_failure(&format!("Cannot create {label}.")),
        })
}

/// Write and sync `contents` to `file`, the new file `create_new` opened at `path`.
/// A partially written file is removed while `file`, and any lock on it, is held.
fn write_new(path: &Path, mut file: &File, contents: &[u8], label: &str) -> Result<()> {
    if file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = std::fs::remove_file(path);
        return Err(io_failure(&format!(
            "Cannot write {label}; the incomplete new file was removed."
        )));
    }
    Ok(())
}

/// Remove the manifest init created at `path`, open as `file`, while it is still empty
/// and still at `path`; a file another writer has put there is kept.
fn remove_created(path: &Path, file: &File) {
    if names(path, file) && file.metadata().is_ok_and(|metadata| metadata.len() == 0) {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether the migrations directory exists. Init never replaces a migrations path
/// that is not a directory.
fn existing_dir(migrations: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(migrations) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(conflict(
            "The migrations path exists but is not a directory; init never replaces it.",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err(io_failure("Cannot inspect the migrations directory.")),
    }
}

/// Run `write` with the migrations directory in place: create it if it is missing and
/// remove it again if `write` fails. Init calls this only while it holds the manifest
/// lock, so the inits of one manifest create, report and remove the directory one at
/// a time, and an init that fails never removes a directory another one reported.
/// Returns whether the directory was created.
fn with_migrations_dir(migrations: &Path, write: impl FnOnce() -> Result<()>) -> Result<bool> {
    let created = !existing_dir(migrations)?;
    if created {
        std::fs::create_dir(migrations).map_err(|_| {
            io_failure("Cannot create the migrations directory; the manifest was not changed.")
        })?;
    }
    if let Err(diagnostic) = write() {
        if created {
            // remove_dir removes only an empty directory: a file added meanwhile stays.
            let _ = std::fs::remove_dir(migrations);
        }
        return Err(diagnostic);
    }
    Ok(created)
}

/// Replace the manifest locked through `held` with `combined` (its exact captured
/// bytes plus the new section) by renaming a synced sibling file over it, so a
/// failure leaves the original intact. The caller keeps the lock until this returns.
/// A manifest the caller cannot write, or one replaced or edited since capture, aborts.
fn replace_manifest(
    path: &Path,
    held: &File,
    original: &[u8],
    combined: &[u8],
    before_write: impl FnOnce(),
) -> Result<()> {
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
    // From here the synced copy is removed by hand on failure; `keep` also clears the
    // temporary-file attribute Windows gives it.
    let temp = temp.into_temp_path().keep().map_err(|_| failed())?;
    let replaced = (|| {
        // The lock excludes other inits, not an editor that ignores it. Replace only
        // the locked file, still at `path`, with exactly the bytes that were validated.
        if !names(path, held) || reread(held).as_deref() != Some(original) {
            return Err(conflict(CHANGED));
        }
        before_write();
        // Unlike tempfile's persist, std's rename also replaces a file that is still
        // open (here through the lock) on Windows.
        std::fs::rename(&temp, path).map_err(|_| failed())
    })();
    if replaced.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    replaced
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
    initialize_with(args, File::try_lock, |_| {})
}

/// A race boundary in `initialize_with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// No manifest existed; init is about to create it.
    Create,
    /// Init holds the manifest lock and is about to write the manifest.
    Write,
}

/// `initialize`, taking the manifest lock with `try_lock` and calling `pause` at each
/// `Step` (tests fail the lock or run a concurrent init there).
fn initialize_with(
    args: &InitArgs,
    try_lock: TryLock,
    mut pause: impl FnMut(Step),
) -> Result<(Value, Vec<String>)> {
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
    // An existing manifest is locked, then captured under the same file rules as
    // every db command; the lock is held until the extended copy has replaced it.
    let held = match std::fs::symlink_metadata(manifest) {
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        _ => Some(inventory::capture(
            manifest,
            input_file::open_replaceable,
            |file| lock(manifest, file, try_lock),
        )?),
    };
    let appended = held.is_some();
    let mut combined = Vec::new();
    if let Some((_, bytes)) = &held {
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
    // Refuse a migrations path that is not a directory before writing anything. The
    // directory itself is created only once init holds the manifest lock.
    existing_dir(&migrations)?;
    let created_dir = match &held {
        Some((file, bytes)) => with_migrations_dir(&migrations, || {
            replace_manifest(manifest, file, bytes, &combined, || pause(Step::Write))
        })?,
        None => {
            pause(Step::Create);
            let file = create_new(
                manifest,
                conflict("The manifest was created by another process while init was running; it was left unchanged."),
                "the manifest",
            )?;
            // A new manifest is locked before it is written, so a concurrent init that
            // finds it never extends a partially written file.
            if let Err(error) = lock(manifest, &file, try_lock) {
                // On a conflict, another init that found the file holds its lock and
                // extends it. Otherwise the empty file is still this init's own.
                if error.code != "DB_INIT_CONFLICT" {
                    remove_created(manifest, &file);
                }
                return Err(error);
            }
            with_migrations_dir(&migrations, || {
                pause(Step::Write);
                write_new(manifest, &file, &combined, "the manifest")
            })
            .inspect_err(|_| remove_created(manifest, &file))?
        }
    };
    // Any cleanup above ran under the lock; release it only now.
    drop(held);

    let declared = document
        .get("databases")
        .and_then(toml::Value::as_table)
        .map_or(0, |databases| databases.len());
    let flags = selection_flags(manifest, (declared > 1).then_some(name));
    let steps = next_steps(args.engine, name, &secrets, &flags);
    let mut lines = vec![
        if appended {
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
            if created_dir {
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
            "action": if appended { "appended" } else { "created" },
        },
        "migrations_dir": {
            "path": migrations.display().to_string(),
            "action": if created_dir { "created" } else { "existing" },
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
    let path = dir.join(&source);
    let file = create_new(
        &path,
        Diagnostic::new(
            "DB_MIGRATION_EXISTS",
            "The next migration file already exists.",
        ),
        "the migration file",
    )?;
    write_new(&path, &file, header.as_bytes(), "the migration file")?;
    let mut shown = project_dir(&args.manifest)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    shown.extend(config.migrations_dir.split('/'));
    shown.push(&source);
    // --database is required whenever several databases are declared; repeat it
    // whenever it was given.
    let flags = selection_flags(
        &args.manifest,
        args.database.is_some().then_some(database.as_str()),
    );
    let lines = vec![
        format!(
            "Created {} for database {database} ({}).",
            shown.display(),
            config.engine
        ),
        format!(
            "Write its SQL, then review it with `ntnt db plan --target <target>{flags} --out review.json`."
        ),
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

    /// The manifest locked and captured as init does it.
    fn capture_locked(path: &Path) -> (File, Vec<u8>) {
        inventory::capture(path, input_file::open_replaceable, |file| {
            lock(path, file, File::try_lock)
        })
        .unwrap()
    }

    #[test]
    fn manifest_replacement_never_loses_a_concurrent_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        // init validated "[lint]\n"; the file now holds a different edit.
        std::fs::write(&path, "[lint]\nstrict = true\n").unwrap();
        let (held, _) = capture_locked(&path);
        let error = replace_manifest(
            &path,
            &held,
            b"[lint]\n",
            b"[lint]\n\n[secrets.APP_DATABASE]\n",
            || {},
        )
        .unwrap_err();
        assert_eq!(error.code, "DB_INIT_CONFLICT");
        // Windows locks are mandatory: read the file once the lock is released.
        drop(held);
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

    /// A lock on a file that is no longer the manifest protects nothing: an editor
    /// saved by renaming a new file over the manifest after init captured it.
    #[test]
    fn manifest_replacement_aborts_when_the_locked_file_was_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, "[lint]\n").unwrap();
        let (held, original) = capture_locked(&path);
        let saved = dir.path().join("saved.toml");
        std::fs::write(&saved, "[lint]\nstrict = true\n").unwrap();
        std::fs::rename(&saved, &path).unwrap();
        let error = replace_manifest(
            &path,
            &held,
            &original,
            b"[lint]\n\n[secrets.APP_DATABASE]\n",
            || {},
        )
        .unwrap_err();
        assert_eq!(error.code, "DB_INIT_CONFLICT");
        drop(held);
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

    /// Another writer replaced the manifest between init opening and locking it.
    #[test]
    fn a_lock_on_a_replaced_manifest_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, "[lint]\n").unwrap();
        let opened = input_file::open_replaceable(&path).unwrap();
        let saved = dir.path().join("saved.toml");
        std::fs::write(&saved, "[lint]\nstrict = true\n").unwrap();
        std::fs::rename(&saved, &path).unwrap();
        assert_eq!(
            lock(&path, &opened, File::try_lock).unwrap_err().code,
            "DB_INIT_CONFLICT"
        );
    }

    /// The Linux NFS client emulates `flock` with a byte-range lock on the whole file,
    /// which is refused (`EBADF`) on a descriptor not open for writing. Every Linux
    /// filesystem applies that rule to `fcntl` locks, so the manifest handle init
    /// locks must take an exclusive one.
    #[cfg(unix)]
    #[test]
    fn the_manifest_handle_takes_an_exclusive_byte_range_lock() {
        use std::os::fd::AsRawFd;
        fn byte_range_lock(file: &File) -> std::io::Result<()> {
            // SAFETY: an all-zero flock is a valid value; the fields are set below.
            let mut range: libc::flock = unsafe { std::mem::zeroed() };
            range.l_type = libc::F_WRLCK as libc::c_short;
            range.l_whence = libc::SEEK_SET as libc::c_short;
            // l_start = l_len = 0: the whole file.
            // SAFETY: a live descriptor and a valid flock that outlives the call.
            match unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &range) } {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, "[lint]\n").unwrap();
        let read_only = File::open(&path).unwrap();
        assert_eq!(
            byte_range_lock(&read_only).unwrap_err().raw_os_error(),
            Some(libc::EBADF),
            "a read-only descriptor cannot take an exclusive byte-range lock"
        );
        drop(read_only);
        byte_range_lock(&input_file::open_replaceable(&path).unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn manifest_replacement_keeps_the_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, "[lint]\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let (held, original) = capture_locked(&path);
        replace_manifest(
            &path,
            &held,
            &original,
            b"[lint]\n\n[secrets.APP_DATABASE]\n",
            || {},
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[lint]\n\n[secrets.APP_DATABASE]\n"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    fn sqlite_init(manifest: &Path, database: &str) -> InitArgs {
        InitArgs {
            engine: Engine::Sqlite,
            database: database.into(),
            manifest: manifest.to_path_buf(),
            json: true,
        }
    }

    /// Init "analytics" runs to completion when init "app" reaches `step` on the same
    /// manifest. Whatever each reports, no successful declaration may be lost, and the
    /// migrations directory a successful init reports must still exist.
    fn race(existing: Option<&str>, step: Step) -> [Result<(Value, Vec<String>)>; 2] {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("ntnt.toml");
        if let Some(text) = existing {
            std::fs::write(&manifest, text).unwrap();
        }
        let mut second = None;
        let first = initialize_with(&sqlite_init(&manifest, "app"), File::try_lock, |at| {
            if at == step {
                second = Some(initialize(&sqlite_init(&manifest, "analytics")));
            }
        });
        let second = second.expect("the second init ran at the race boundary");
        let document = CapturedInputs::parse(&std::fs::read(&manifest).unwrap()).unwrap();
        for (name, result) in [("app", &first), ("analytics", &second)] {
            if let Ok((report, _)) = result {
                assert!(
                    document["databases"].get(name).is_some(),
                    "the successful init of {name} was discarded:\n{document}"
                );
                assert!(
                    dir.path().join(MIGRATIONS_DIR).is_dir(),
                    "the migrations directory the successful init of {name} reported ({}) is gone",
                    report["migrations_dir"]["action"]
                );
            }
        }
        [first, second]
    }

    #[test]
    fn concurrent_init_never_discards_a_declaration_appended_to_a_manifest() {
        let [first, second] = race(Some("[lint]\nstrict = true\n"), Step::Write);
        assert!(first.is_ok(), "{:?}", first.err());
        assert_eq!(second.unwrap_err().code, "DB_INIT_CONFLICT");
    }

    #[test]
    fn concurrent_init_never_discards_a_declaration_in_a_new_manifest() {
        let [first, second] = race(None, Step::Write);
        assert!(first.is_ok(), "{:?}", first.err());
        assert_eq!(second.unwrap_err().code, "DB_INIT_CONFLICT");
    }

    /// Init "app" found no manifest; init "analytics" creates the manifest and the
    /// migrations directory before "app" creates the manifest. "app" loses and must
    /// leave the directory "analytics" reported.
    #[test]
    fn a_losing_init_never_removes_the_migrations_directory_of_the_winner() {
        let [first, second] = race(None, Step::Create);
        assert_eq!(first.unwrap_err().code, "DB_INIT_CONFLICT");
        let (report, _) = second.unwrap();
        assert_eq!(report["migrations_dir"]["action"], "created");
    }

    /// A lock call that fails outright (a filesystem answering `flock` with `ENOLCK`)
    /// leaves nothing behind: the manifest init just created is still its own, empty
    /// file. Only contention keeps it: the other init holding the lock extends it.
    #[test]
    fn a_failed_lock_on_a_new_manifest_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("ntnt.toml");
        let unsupported = |_: &File| Err(TryLockError::Error(ErrorKind::Unsupported.into()));
        let error =
            initialize_with(&sqlite_init(&manifest, "app"), unsupported, |_| {}).unwrap_err();
        assert_eq!(error.code, "DB_SCAFFOLD_IO");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "a failed init left files behind"
        );

        let held = |_: &File| Err(TryLockError::WouldBlock);
        let error = initialize_with(&sqlite_init(&manifest, "app"), held, |_| {}).unwrap_err();
        assert_eq!(error.code, "DB_INIT_CONFLICT");
        assert!(manifest.is_file(), "the lock holder's manifest was removed");
        assert!(!dir.path().join(MIGRATIONS_DIR).exists());
    }

    #[test]
    fn printed_arguments_are_single_posix_shell_words() {
        for (value, word) in [
            ("ntnt.toml", "ntnt.toml"),
            ("config/db-1.toml", "config/db-1.toml"),
            ("my project/ntnt.toml", "'my project/ntnt.toml'"),
            ("team's $HOME", r"'team'\''s $HOME'"),
            ("", "''"),
        ] {
            assert_eq!(shell_word(value), word);
        }
        assert_eq!(selection_flags(Path::new("ntnt.toml"), None), "");
        assert_eq!(
            selection_flags(Path::new("a b/ntnt.toml"), Some("analytics")),
            " --manifest 'a b/ntnt.toml' --database analytics"
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
