//! First-time setup: `ntnt db init` declares one logical database and its secret
//! names in ntnt.toml; `ntnt db new` creates the next numbered SQL migration.
//! Neither command reads a secret value, connects to a database or replaces a file.
//! Both hold an exclusive lock on ntnt.toml while they work and refuse, never
//! waiting, while another one holds it.
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

/// `path` for a diagnostic message, as one POSIX shell word like the printed commands.
fn shown(path: &Path) -> String {
    shell_word(&path.to_string_lossy())
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
/// `review` names the plan file for the first migration, which `ntnt db new`
/// prints too.
fn next_steps(
    engine: Engine,
    name: &str,
    secrets: &[String; 3],
    flags: &str,
    review: &str,
) -> Vec<String> {
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
        format!("Run `ntnt db status --target development{flags}`, then `ntnt db plan --target development{flags} --out {review}`."),
        format!("Review {review} and the SQL, set {approval} to its plan_digest, then run `ntnt db migrate --target development{flags} --plan {review}`."),
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

/// How init and new attempt the manifest lock: `File::try_lock`, replaced in tests.
type TryLock = fn(&File) -> std::result::Result<(), TryLockError>;

/// Why the manifest lock was not taken.
enum Refusal {
    /// The path no longer names the opened file.
    Replaced,
    /// Another handle holds the lock.
    Held,
    /// The lock call itself failed.
    Failed(std::io::Error),
}

/// Take the exclusive manifest lock on `file`, the manifest at `path`, without
/// waiting. Every init holds it from capturing the manifest until its write is
/// complete, and every new from capturing it until the new migration file exists.
/// A concurrent init or new refuses instead, so init never replaces the manifest
/// with stale bytes and two new runs never take the same migration ID.
fn try_lock_manifest(
    path: &Path,
    file: &File,
    try_lock: TryLock,
) -> std::result::Result<(), Refusal> {
    let locked = try_lock(file);
    // A lock on a file that was replaced meanwhile protects nothing.
    if !names(path, file) {
        return Err(Refusal::Replaced);
    }
    match locked {
        Ok(()) => Ok(()),
        Err(TryLockError::WouldBlock) => Err(Refusal::Held),
        Err(TryLockError::Error(error)) => Err(Refusal::Failed(error)),
    }
}

/// Take init's manifest lock (`try_lock_manifest`).
fn lock(path: &Path, file: &File, try_lock: TryLock) -> Result<()> {
    try_lock_manifest(path, file, try_lock).map_err(|refusal| match refusal {
        Refusal::Replaced => conflict(CHANGED),
        Refusal::Held => conflict(&format!(
            "Another `ntnt db init` or `ntnt db new` holds the lock on the manifest {}; it was left unchanged.",
            shown(path)
        )),
        Refusal::Failed(error) => io_failure(&format!(
            "Cannot lock the manifest {} for an exclusive update: {error}; it was left unchanged.",
            shown(path)
        )),
    })
}

/// Take new's manifest lock (`try_lock_manifest`).
fn lock_numbering(path: &Path, file: &File, try_lock: TryLock) -> Result<()> {
    let busy = |message: String| Diagnostic::new("DB_NEW_CONFLICT", &message);
    try_lock_manifest(path, file, try_lock).map_err(|refusal| match refusal {
        Refusal::Replaced => busy(format!(
            "The manifest {} was replaced while `ntnt db new` was locking it; no migration was created.",
            shown(path)
        )),
        Refusal::Held => busy(format!(
            "Another `ntnt db new` or `ntnt db init` holds the lock on the manifest {}; no migration was created.",
            shown(path)
        )),
        Refusal::Failed(error) => io_failure(&format!(
            "Cannot lock the manifest {} to number the new migration: {error}; no migration was created.",
            shown(path)
        )),
    })
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
            _ => io_failure(&format!("Cannot create {label} {}: {error}.", shown(path))),
        })
}

/// Write and sync `contents` to `file`, the new file `create_new` opened at `path`.
/// A partially written file is removed while `file`, and any lock on it, is held,
/// and the diagnostic says whether that removal worked.
fn write_new(path: &Path, mut file: &File, contents: &[u8], label: &str) -> Result<()> {
    let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) else {
        return Ok(());
    };
    let cleanup = match std::fs::remove_file(path) {
        Ok(()) => "the incomplete new file was removed".to_string(),
        Err(cleanup) => format!(
            "the incomplete new file could not be removed ({cleanup}); delete it before running ntnt db again"
        ),
    };
    Err(io_failure(&format!(
        "Cannot write {label} {}: {error}; {cleanup}.",
        shown(path)
    )))
}

/// Remove the manifest init created at `path`, open as `file`, while it is still empty
/// and still at `path`; a file another writer has put there is kept. An error means
/// the empty file is still there.
fn remove_created(path: &Path, file: &File) -> std::io::Result<()> {
    if names(path, file) && file.metadata().is_ok_and(|metadata| metadata.len() == 0) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// `error`, noting that the empty manifest init created at `path` is still there when
/// `removed` failed.
fn with_cleanup(mut error: Diagnostic, path: &Path, removed: std::io::Result<()>) -> Diagnostic {
    if let Err(cleanup) = removed {
        error.message = format!(
            "{} However, the empty manifest {} it created could not be removed ({cleanup}); delete it before running ntnt db again.",
            error.message,
            shown(path)
        );
    }
    error
}

/// Whether the migrations directory exists. Init never replaces a migrations path
/// that is not a directory.
fn existing_dir(migrations: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(migrations) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(conflict(&format!(
            "The migrations path {} exists but is not a directory; init never replaces it.",
            shown(migrations)
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_failure(&format!(
            "Cannot inspect the migrations directory {}: {error}.",
            shown(migrations)
        ))),
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
        std::fs::create_dir(migrations).map_err(|error| {
            io_failure(&format!(
                "Cannot create the migrations directory {}: {error}; the manifest was not changed.",
                shown(migrations)
            ))
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
    let failed = |error: std::io::Error| {
        io_failure(&format!(
            "Cannot write the extended manifest {}: {error}; the original was left unchanged.",
            shown(path)
        ))
    };
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
        .map_err(|error| {
            io_failure(&format!(
                "The manifest {} is not writable: {error}; it was left unchanged.",
                shown(path)
            ))
        })?
        .permissions();
    let mut temp = tempfile::Builder::new()
        .prefix(".ntnt.toml.")
        .suffix(".tmp")
        .tempfile_in(project_dir(path).unwrap_or(Path::new(".")))
        .map_err(failed)?;
    temp.write_all(combined)
        .and_then(|()| temp.as_file().set_permissions(permissions))
        .and_then(|()| temp.as_file().sync_all())
        .map_err(failed)?;
    // From here the synced copy is removed by hand on failure; `keep` also clears the
    // temporary-file attribute Windows gives it.
    let temp = temp
        .into_temp_path()
        .keep()
        .map_err(|error| failed(error.error))?;
    let replaced = (|| {
        // The lock excludes other inits, not an editor that ignores it. Replace only
        // the locked file, still at `path`, with exactly the bytes that were validated.
        if !names(path, held) || reread(held).as_deref() != Some(original) {
            return Err(conflict(CHANGED));
        }
        before_write();
        // Unlike tempfile's persist, std's rename also replaces a file that is still
        // open (here through the lock) on Windows.
        std::fs::rename(&temp, path).map_err(failed)
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
            inventory::Input::Manifest(manifest),
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
    CapturedInputs::endpoint(&config, name, "development")?;
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
                // On a conflict, another command that found the file holds its lock:
                // an init extends it, and a new finds no database in it and leaves it
                // empty for the next init to extend. Either way it is no longer only
                // this init's file to remove. Otherwise the empty file is still its own.
                if error.code == "DB_INIT_CONFLICT" {
                    return Err(error);
                }
                return Err(with_cleanup(
                    error,
                    manifest,
                    remove_created(manifest, &file),
                ));
            }
            with_migrations_dir(&migrations, || {
                pause(Step::Write);
                write_new(manifest, &file, &combined, "the manifest")
            })
            .map_err(|error| with_cleanup(error, manifest, remove_created(manifest, &file)))?
        }
    };
    // Any cleanup above ran under the lock; release it only now.
    drop(held);

    let declared = document
        .get("databases")
        .and_then(toml::Value::as_table)
        .map_or(0, |databases| databases.len());
    let flags = selection_flags(manifest, (declared > 1).then_some(name));
    // The plan file the first migration's plan command names (#425).
    let review = next_id(&migrations).map_or_else(
        |_| "review-0001.json".to_owned(),
        |(id, width)| review_file(id, width),
    );
    let steps = next_steps(args.engine, name, &secrets, &flags, &review);
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
/// database. It reads only the manifest and the migration directory listing, and
/// holds the manifest lock while it numbers and creates the file.
pub(super) fn new(args: &NewArgs) -> anyhow::Result<()> {
    finish("new", args.json, create_migration(args))
}

/// The ID after the highest `<digits>_<name>.sql|.toml` entry, and the width of the
/// widest existing ID, so a new file keeps the directory's zero padding: a history
/// numbered `001_` to `034_` gets `035_`, which `ls` and shell globs list after it
/// (#431). An empty directory starts at four digits.
fn next_id(dir: &Path) -> Result<(u64, usize)> {
    let unreadable = |error: std::io::Error| {
        Diagnostic::new(
            "DB_CONFIG",
            &format!(
                "Cannot enumerate the migration directory {}: {error}.",
                shown(dir)
            ),
        )
    };
    let (mut highest, mut width) = (0u64, 0usize);
    for entry in std::fs::read_dir(dir).map_err(unreadable)? {
        let name = entry.map_err(unreadable)?.file_name();
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
    Ok((next, if width == 0 { 4 } else { width }))
}

/// The plan file for migration `id`, written `width` digits wide as its file
/// name is: a new file for each migration, since `--out` never replaces one
/// (#425).
fn review_file(id: u64, width: usize) -> String {
    format!("review-{id:0width$}.json")
}

fn create_migration(args: &NewArgs) -> Result<(Value, Vec<String>)> {
    create_migration_with(args, File::try_lock, |_| {})
}

/// A race boundary in `create_migration_with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NewStep {
    /// New holds the manifest lock and is about to list the migrations directory.
    List,
    /// New has chosen the next ID and is about to create its file.
    Create,
}

/// `create_migration`, taking the manifest lock with `try_lock` and calling `pause`
/// at each `NewStep` (tests fail the lock or run a concurrent command there).
fn create_migration_with(
    args: &NewArgs,
    try_lock: TryLock,
    mut pause: impl FnMut(NewStep),
) -> Result<(Value, Vec<String>)> {
    if !valid_migration_name(&args.name) {
        return Err(Diagnostic::new(
            "DB_MIGRATION_NAME",
            "Invalid migration name.",
        ));
    }
    // The manifest is locked as init locks it, then captured under the same file
    // rules as every db command. The lock is held until the new file exists, so a
    // concurrent new or init refuses: new runs number their files one at a time,
    // each after the files the runs before it created. Nothing is written through
    // this handle. It is open for writing because NFS locks only such a handle; a
    // manifest new may not write is opened read-only, which local filesystems lock
    // as well and NFS refuses.
    let manifest = args.manifest.as_path();
    let input = inventory::Input::Manifest(manifest);
    let (held, bytes) = inventory::capture(input, input_file::open_replaceable, |file| {
        lock_numbering(manifest, file, try_lock)
    })?;
    let document = CapturedInputs::parse(&bytes)?;
    let (database, config) = CapturedInputs::select(&document, args.database.as_deref())?;
    let dir = CapturedInputs::migrations_dir(&args.manifest, &config)?;
    pause(NewStep::List);
    let (id, width) = next_id(&dir)?;
    let source = format!("{id:0width$}_{}.sql", args.name);
    let header = format!(
        "-- {source}: database \"{database}\" ({engine}).\n\
         -- Write the SQL below. Once this migration has been applied anywhere,\n\
         -- never edit or renumber it; add a new migration instead.\n",
        engine = config.engine
    );
    let path = dir.join(&source);
    pause(NewStep::Create);
    let file = create_new(
        &path,
        Diagnostic::new(
            "DB_MIGRATION_EXISTS",
            &format!("The next migration file {} already exists.", shown(&path)),
        ),
        "the migration file",
    )?;
    write_new(&path, &file, header.as_bytes(), "the migration file")?;
    // The new file is complete; release the lock only now. An early return releases
    // it after `write_new` has removed an incomplete file.
    drop(held);
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
            "Write its SQL, then review it with `ntnt db plan --target <target>{flags} --out {}`.",
            review_file(id, width)
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

    /// A lock that fails for a reason other than contention names the manifest and
    /// the OS error (REVIEW.md: errors carry paths).
    #[test]
    fn a_failed_lock_names_the_manifest_and_the_os_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, b"").unwrap();
        let file = File::open(&path).unwrap();
        let error = lock(&path, &file, |_| {
            Err(TryLockError::Error(std::io::Error::other(
                "no locks available",
            )))
        })
        .unwrap_err();
        assert_eq!(error.code, "DB_SCAFFOLD_IO");
        assert!(error.message.contains(&shown(&path)), "{}", error.message);
        assert!(
            error.message.contains("no locks available"),
            "{}",
            error.message
        );
    }

    /// Migrations-directory failures name the directory, and I/O failures the OS error.
    #[test]
    fn migrations_directory_errors_name_the_path_and_the_os_error() {
        let dir = tempfile::tempdir().unwrap();
        // A missing parent: nothing is there to inspect, and creating the directory fails.
        let orphan = dir.path().join("missing").join("migrations");
        let error = with_migrations_dir(&orphan, || Ok(())).unwrap_err();
        assert_eq!(error.code, "DB_SCAFFOLD_IO");
        assert!(
            error.message.contains(&format!(
                "Cannot create the migrations directory {}: ",
                shown(&orphan)
            )),
            "{}",
            error.message
        );
        assert!(error.message.contains("(os error"), "{}", error.message);

        // A file where the parent should be: inspecting fails on Unix (ENOTDIR);
        // elsewhere the path reads as missing and creating it fails.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let blocked = blocker.join("migrations");
        let error = with_migrations_dir(&blocked, || Ok(())).unwrap_err();
        assert_eq!(error.code, "DB_SCAFFOLD_IO");
        assert!(
            error.message.contains(&shown(&blocked)),
            "{}",
            error.message
        );
        assert!(error.message.contains("(os error"), "{}", error.message);
        #[cfg(unix)]
        assert!(
            error
                .message
                .starts_with("Cannot inspect the migrations directory"),
            "{}",
            error.message
        );

        // A file at the migrations path itself is refused by name.
        let error = existing_dir(&blocker).unwrap_err();
        assert_eq!(error.code, "DB_INIT_CONFLICT");
        assert!(
            error.message.contains(&shown(&blocker)),
            "{}",
            error.message
        );
    }

    /// Replacing an existing manifest names it and the OS error when the writability
    /// probe or the write itself fails, and leaves the original in place.
    #[test]
    fn manifest_replacement_errors_name_the_manifest_and_the_os_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        let original: &[u8] = b"[lint]\n";
        // The probe: a manifest that cannot be opened for writing.
        let other = dir.path().join("other.toml");
        std::fs::write(&other, original).unwrap();
        let held = File::open(&other).unwrap();
        let error = replace_manifest(&path, &held, original, original, || {}).unwrap_err();
        assert!(
            error
                .message
                .contains(&format!("The manifest {} is not writable: ", shown(&path))),
            "{}",
            error.message
        );
        assert!(error.message.contains("(os error"), "{}", error.message);

        // The write: a directory put in the manifest's place after the checks makes
        // the rename fail. Unix only: Windows cannot replace a file that is still open.
        #[cfg(unix)]
        {
            std::fs::write(&path, original).unwrap();
            let held = File::open(&path).unwrap();
            let error =
                replace_manifest(&path, &held, original, b"[lint]\nstrict = true\n", || {
                    std::fs::remove_file(&path).unwrap();
                    std::fs::create_dir(&path).unwrap();
                })
                .unwrap_err();
            assert_eq!(error.code, "DB_SCAFFOLD_IO");
            assert!(
                error.message.contains(&format!(
                    "Cannot write the extended manifest {}: ",
                    shown(&path)
                )),
                "{}",
                error.message
            );
            assert!(error.message.contains("(os error"), "{}", error.message);
        }
    }

    /// `ntnt db new` names a migration directory it cannot list, and the OS error.
    #[test]
    fn an_unlistable_migration_directory_is_named_with_the_os_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("migrations");
        let error = next_id(&missing).unwrap_err();
        assert_eq!(error.code, "DB_CONFIG");
        assert!(
            error.message.contains(&shown(&missing)),
            "{}",
            error.message
        );
        assert!(error.message.contains("(os error"), "{}", error.message);
    }

    /// Creation failures name the file and the OS error (REVIEW.md: errors carry paths).
    #[test]
    fn creation_errors_name_the_file_and_the_os_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("0001_initial.sql");
        let error = create_new(&path, conflict("exists"), "the migration file").unwrap_err();
        assert_eq!(error.code, "DB_SCAFFOLD_IO");
        assert!(error.message.contains(&shown(&path)), "{}", error.message);
        assert!(error.message.contains("(os error"), "{}", error.message);
    }

    /// A failed write names the file and says whether the incomplete file was removed.
    #[test]
    fn write_failures_say_whether_the_incomplete_file_was_removed() {
        let dir = tempfile::tempdir().unwrap();
        // Writing through a read-only handle fails on every platform.
        let other = dir.path().join("other");
        std::fs::write(&other, b"").unwrap();
        let read_only = File::open(&other).unwrap();

        let removable = dir.path().join("0001_initial.sql");
        std::fs::write(&removable, b"").unwrap();
        let error =
            write_new(&removable, &read_only, b"-- one\n", "the migration file").unwrap_err();
        assert!(!removable.exists());
        assert!(
            error.message.contains(&shown(&removable)),
            "{}",
            error.message
        );
        assert!(error.message.contains("was removed"), "{}", error.message);

        // remove_file refuses a directory, so this cleanup fails.
        let stuck = dir.path().join("0002_names.sql");
        std::fs::create_dir(&stuck).unwrap();
        let error = write_new(&stuck, &read_only, b"-- two\n", "the migration file").unwrap_err();
        assert!(stuck.exists());
        assert!(
            error.message.contains("could not be removed"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("was removed"), "{}", error.message);
    }

    /// When init cannot remove the empty manifest it created, the diagnostic says so.
    #[test]
    fn a_manifest_that_could_not_be_removed_is_reported() {
        let path = Path::new("ntnt.toml");
        let removed = with_cleanup(io_failure("Cannot lock."), path, Ok(()));
        assert_eq!(removed.message, "Cannot lock.");
        let stuck = with_cleanup(
            io_failure("Cannot lock."),
            path,
            Err(std::io::Error::other("read-only file system")),
        );
        assert!(
            stuck
                .message
                .contains("ntnt.toml it created could not be removed (read-only file system)"),
            "{}",
            stuck.message
        );
    }

    /// remove_created returns a failed removal instead of discarding it.
    #[cfg(unix)]
    #[test]
    fn remove_created_returns_a_failed_removal() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return; // root ignores directory permissions, so the removal cannot fail
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        let file = File::create_new(&path).unwrap();
        let permissions =
            |mode| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode));
        permissions(0o555).unwrap();
        let removed = remove_created(&path, &file);
        permissions(0o755).unwrap();
        assert!(
            removed.is_err(),
            "removal in a read-only directory succeeded"
        );
        assert!(path.exists());
        assert!(remove_created(&path, &file).is_ok());
        assert!(!path.exists());
    }

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
        inventory::capture(
            inventory::Input::Manifest(path),
            input_file::open_replaceable,
            |file| lock(path, file, File::try_lock),
        )
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

    fn new_migration(manifest: &Path, name: &str) -> NewArgs {
        NewArgs {
            name: name.into(),
            manifest: manifest.to_path_buf(),
            database: None,
            json: true,
        }
    }

    /// The sorted file names in `dir`.
    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// `new accounts` runs `new events` to completion when it reaches `step` on a
    /// scaffolded manifest. The first creates the only migration with ID 1 whatever
    /// the second reports, and once it has returned the next `new` takes ID 2.
    /// Returns the manifest path and what the second reported.
    fn new_race(step: NewStep) -> (PathBuf, Result<(Value, Vec<String>)>) {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("ntnt.toml");
        initialize(&sqlite_init(&manifest, "app")).unwrap();
        let mut second = None;
        let first = create_migration_with(
            &new_migration(&manifest, "accounts"),
            File::try_lock,
            |at| {
                if at == step {
                    second = Some(create_migration(&new_migration(&manifest, "events")));
                }
            },
        );
        let second = second.expect("the second new ran at the race boundary");
        let (report, _) = first.unwrap_or_else(|error| panic!("the first new failed: {error}"));
        assert_eq!(report["migration"]["source"], "0001_accounts.sql");
        let migrations = dir.path().join(MIGRATIONS_DIR);
        assert_eq!(
            file_names(&migrations),
            ["0001_accounts.sql"],
            "the second new reported {second:?}"
        );
        // The first new released the lock when it returned.
        let (report, _) = create_migration(&new_migration(&manifest, "events")).unwrap();
        assert_eq!(report["migration"]["source"], "0002_events.sql");
        (manifest, second)
    }

    /// The manifest-lock refusal of `ntnt db new`: it names the manifest and the
    /// commands that take the lock.
    fn assert_new_conflict(error: &Diagnostic, manifest: &Path) {
        assert_eq!(error.code, "DB_NEW_CONFLICT", "{error}");
        assert!(error.message.contains(&shown(manifest)), "{error}");
        assert!(
            error.message.contains("`ntnt db new` or `ntnt db init`"),
            "{error}"
        );
    }

    /// Two `new` runs never take the same ID: one that starts after another has
    /// chosen its ID but before it has created that file refuses at once.
    #[test]
    fn concurrent_new_never_takes_the_id_another_new_is_creating() {
        let (manifest, second) = new_race(NewStep::Create);
        assert_new_conflict(&second.unwrap_err(), &manifest);
    }

    /// `new` locks the manifest before it lists the migrations directory.
    #[test]
    fn concurrent_new_refuses_before_another_new_lists_the_migrations() {
        let (manifest, second) = new_race(NewStep::List);
        assert_new_conflict(&second.unwrap_err(), &manifest);
    }

    /// `new` pads the next ID to the widest existing ID, so a history numbered
    /// with three digits keeps three; an empty directory starts at four. The
    /// plan file it names has the same width (#431).
    #[test]
    fn new_keeps_the_width_of_the_widest_existing_id() {
        let cases: [(&[&str], &str, &str); 5] = [
            (
                &["001_first.sql", "034_last.sql"],
                "035_next.sql",
                "review-035.json",
            ),
            (
                &["0001_first.sql", "0034_last.sql"],
                "0035_next.sql",
                "review-0035.json",
            ),
            (&[], "0001_next.sql", "review-0001.json"),
            (
                &["001_first.sql", "0002_second.sql"],
                "0003_next.sql",
                "review-0003.json",
            ),
            (
                &["998_first.sql", "999_last.sql"],
                "1000_next.sql",
                "review-1000.json",
            ),
        ];
        for (existing, created, review) in cases {
            let dir = tempfile::tempdir().unwrap();
            let manifest = dir.path().join("ntnt.toml");
            initialize(&sqlite_init(&manifest, "app")).unwrap();
            let migrations = dir.path().join(MIGRATIONS_DIR);
            for name in existing {
                std::fs::write(migrations.join(name), "SELECT 1;\n").unwrap();
            }
            let (report, lines) = create_migration(&new_migration(&manifest, "next"))
                .unwrap_or_else(|error| panic!("{existing:?}: {error}"));
            assert_eq!(report["migration"]["source"], created, "{existing:?}");
            assert!(migrations.join(created).is_file(), "{existing:?}");
            assert_eq!(file_names(&migrations).len(), existing.len() + 1);
            assert!(
                lines[1].contains(&format!("--out {review}`")),
                "{existing:?}: {lines:?}"
            );
        }
    }

    /// A manifest declaring `app` with its own migrations directory, so that init
    /// can add a database that uses `migrations/`.
    fn manifest_with_app(dir: &Path) -> PathBuf {
        let manifest = dir.join("ntnt.toml");
        std::fs::write(
            &manifest,
            "[databases.app]\nengine = \"sqlite\"\nschema = \"main\"\nmigrations_dir = \"app\"\n\
             [databases.app.targets.development]\nconnection_secret = \"APP_DATABASE_FILE\"\n\
             [secrets.APP_DATABASE_FILE]\n",
        )
        .unwrap();
        std::fs::create_dir(dir.join("app")).unwrap();
        manifest
    }

    /// `new` and `init` exclude each other through the manifest lock: an init that
    /// starts while a `new` holds it refuses and leaves the manifest unchanged.
    #[test]
    fn init_refuses_while_new_holds_the_manifest_lock() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = manifest_with_app(dir.path());
        let before = std::fs::read(&manifest).unwrap();
        let mut init = None;
        let created = create_migration_with(
            &new_migration(&manifest, "accounts"),
            File::try_lock,
            |at| {
                if at == NewStep::Create {
                    init = Some(initialize(&sqlite_init(&manifest, "analytics")));
                }
            },
        );
        let init = init.expect("init ran while new held the lock");
        assert_eq!(
            created.unwrap().0["migration"]["source"],
            "0001_accounts.sql"
        );
        let error = init.expect_err("init succeeded while new held the manifest lock");
        assert_eq!(error.code, "DB_INIT_CONFLICT");
        assert!(error.message.contains("`ntnt db new`"), "{error}");
        assert_eq!(std::fs::read(&manifest).unwrap(), before);
        assert!(!dir.path().join(MIGRATIONS_DIR).exists());
    }

    /// A lock call that fails outright (a filesystem answering `flock` with `ENOLCK`)
    /// stops new before it lists the migrations, naming the manifest and the OS
    /// error; contention is refused the same way. Neither creates a file.
    #[test]
    fn new_lists_and_creates_nothing_without_the_manifest_lock() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("ntnt.toml");
        initialize(&sqlite_init(&manifest, "app")).unwrap();
        let failed = |_: &File| {
            Err(TryLockError::Error(std::io::Error::other(
                "no locks available",
            )))
        };
        let held = |_: &File| Err(TryLockError::WouldBlock);
        let mut reached = Vec::new();
        let args = new_migration(&manifest, "accounts");
        let error = create_migration_with(&args, failed, |at| reached.push(at)).unwrap_err();
        assert_eq!(error.code, "DB_SCAFFOLD_IO");
        assert!(error.message.contains(&shown(&manifest)), "{error}");
        assert!(error.message.contains("no locks available"), "{error}");
        let error = create_migration_with(&args, held, |at| reached.push(at)).unwrap_err();
        assert_new_conflict(&error, &manifest);
        assert_eq!(reached, [], "new went on without the lock");
        assert!(file_names(&dir.path().join(MIGRATIONS_DIR)).is_empty());
    }

    /// New refuses a lock on a manifest that was replaced after it opened it: a
    /// concurrent new could lock the replacement and take the same ID.
    #[test]
    fn new_refuses_a_lock_on_a_replaced_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ntnt.toml");
        std::fs::write(&path, "[lint]\n").unwrap();
        let opened = input_file::open_replaceable(&path).unwrap();
        let saved = dir.path().join("saved.toml");
        std::fs::write(&saved, "[lint]\nstrict = true\n").unwrap();
        std::fs::rename(&saved, &path).unwrap();
        let error = lock_numbering(&path, &opened, File::try_lock).unwrap_err();
        assert_eq!(error.code, "DB_NEW_CONFLICT");
        assert!(error.message.contains("was replaced"), "{error}");
        assert!(error.message.contains(&shown(&path)), "{error}");
    }

    /// The Linux NFS client emulates `flock` with a whole-file byte-range lock, which
    /// is refused (`EBADF`) on a descriptor not open for writing. New locks the
    /// manifest through a writable handle that way and leaves the file unchanged. A
    /// manifest new may not write is locked through a read-only handle, which local
    /// filesystems accept and NFS refuses: new then names the manifest and the OS
    /// error and creates nothing.
    #[cfg(unix)]
    #[test]
    fn new_takes_the_manifest_lock_that_nfs_grants() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;
        /// The exclusive whole-file byte-range lock NFS takes for `flock`.
        fn byte_range_lock(file: &File) -> std::result::Result<(), TryLockError> {
            // SAFETY: an all-zero flock is a valid value; the fields are set below.
            let mut range: libc::flock = unsafe { std::mem::zeroed() };
            range.l_type = libc::F_WRLCK as libc::c_short;
            range.l_whence = libc::SEEK_SET as libc::c_short;
            // SAFETY: a live descriptor and a valid flock that outlives the call.
            match unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &range) } {
                -1 => match std::io::Error::last_os_error() {
                    error if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EAGAIN)) => {
                        Err(TryLockError::WouldBlock)
                    }
                    error => Err(TryLockError::Error(error)),
                },
                _ => Ok(()),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("ntnt.toml");
        initialize(&sqlite_init(&manifest, "app")).unwrap();
        let before = std::fs::read(&manifest).unwrap();
        let modified = std::fs::metadata(&manifest).unwrap().modified().unwrap();
        let args = new_migration(&manifest, "accounts");
        let (report, _) = create_migration_with(&args, byte_range_lock, |_| {}).unwrap();
        assert_eq!(report["migration"]["source"], "0001_accounts.sql");
        assert_eq!(std::fs::read(&manifest).unwrap(), before);
        let unchanged = std::fs::metadata(&manifest).unwrap().modified().unwrap();
        assert_eq!(unchanged, modified, "new modified the manifest");

        let mode = |mode| std::fs::set_permissions(&manifest, PermissionsExt::from_mode(mode));
        mode(0o444).unwrap();
        // A process that may write it anyway (root) opens it for writing as before.
        let writable = std::fs::OpenOptions::new()
            .append(true)
            .open(&manifest)
            .is_ok();
        let result =
            create_migration_with(&new_migration(&manifest, "events"), byte_range_lock, |_| {});
        mode(0o644).unwrap();
        let migrations = dir.path().join(MIGRATIONS_DIR);
        if writable {
            assert_eq!(result.unwrap().0["migration"]["source"], "0002_events.sql");
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "DB_SCAFFOLD_IO");
            assert!(error.message.contains(&shown(&manifest)), "{error}");
            assert!(error.message.contains("(os error"), "{error}");
            assert_eq!(file_names(&migrations), ["0001_accounts.sql"]);
        }
    }

    /// A `new` that starts while init holds the manifest lock refuses and creates
    /// nothing.
    #[test]
    fn new_refuses_while_init_holds_the_manifest_lock() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = manifest_with_app(dir.path());
        let mut new = None;
        let mut args = new_migration(&manifest, "accounts");
        args.database = Some("app".into());
        initialize_with(&sqlite_init(&manifest, "analytics"), File::try_lock, |at| {
            if at == Step::Write {
                new = Some(create_migration(&args));
            }
        })
        .unwrap();
        let error = new
            .expect("new ran while init held the lock")
            .expect_err("new succeeded while init held the manifest lock");
        assert_new_conflict(&error, &manifest);
        assert!(file_names(&dir.path().join("app")).is_empty());
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
            let endpoint = CapturedInputs::endpoint(&config, &name, "development").unwrap();
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
