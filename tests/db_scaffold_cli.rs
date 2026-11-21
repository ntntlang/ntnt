//! `ntnt db init` and `ntnt db new` scaffold first-time database setup through
//! the built CLI. They declare names, never secret values, and never replace files.
#[path = "support/bounded_output.rs"]
mod bounded_output;
use bounded_output::BoundedOutput;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

const SQLITE_SECRETS: [&str; 3] = [
    "APP_DATABASE_FILE",
    "APP_REPLAY_FILE",
    "APP_MIGRATION_APPROVAL",
];
const POSTGRES_SECRETS: [&str; 3] = [
    "APP_DATABASE",
    "APP_REPLAY_DATABASE",
    "APP_MIGRATION_APPROVAL",
];

/// A `ntnt db` invocation in `root` with the development env provider and none
/// of the scaffolded secrets set, so nothing can reach a real database.
fn db(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command
        .arg("db")
        .env("NTNT_ENV", "development")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .current_dir(root);
    for name in SQLITE_SECRETS.iter().chain(&POSTGRES_SECRETS) {
        command.env_remove(name);
    }
    command
}

fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "expected a JSON report\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn pass(root: &Path, args: &[&str]) -> Value {
    let output = db(root).args(args).arg("--json").bounded_output();
    let report = report(&output);
    assert!(output.status.success(), "{report}");
    assert_eq!(report["outcome"], "pass", "{report}");
    report
}

fn refused(root: &Path, args: &[&str], code: &str) -> Value {
    let output = db(root).args(args).arg("--json").bounded_output();
    let report = report(&output);
    assert!(!output.status.success(), "{report}");
    assert_eq!(report["outcome"], "fail", "{report}");
    assert_eq!(report["diagnostics"][0]["code"], code, "{report}");
    report
}

/// First diagnostic of `ntnt db status` for the scaffolded development target.
fn status_code(root: &Path, args: &[&str]) -> Value {
    let output = db(root)
        .args(["status", "--target", "development", "--json"])
        .args(args)
        .bounded_output();
    assert!(!output.status.success());
    report(&output)["diagnostics"][0]["code"].clone()
}

/// The `ntnt db ...` commands that printed text quotes in backticks.
fn commands(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|command| command.starts_with("ntnt db "))
        .map(str::to_owned)
        .collect()
}

fn manifest(root: &Path) -> toml::Value {
    std::fs::read_to_string(root.join("ntnt.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

/// Every file below `dir` with its exact bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if entry.file_type().unwrap().is_dir() {
            for (child, bytes) in snapshot(&entry.path()) {
                files.insert(format!("{name}/{child}"), bytes);
            }
        } else {
            files.insert(name, std::fs::read(entry.path()).unwrap());
        }
    }
    files
}

/// Declarations carry names only: no credentials, connection strings or paths.
fn assert_names_only(text: &str, secrets: &[&str]) {
    let lower = text.to_ascii_lowercase();
    for forbidden in [
        "password", "://", "host=", "user=", "dbname=", "sslmode=", ".sqlite",
    ] {
        assert!(!lower.contains(forbidden), "{forbidden:?} in:\n{text}");
    }
    let document: toml::Value = text.parse().unwrap();
    let declared = document["secrets"].as_table().unwrap();
    for name in secrets {
        assert_eq!(
            declared[*name].as_table().map(|t| t.len()),
            Some(0),
            "[secrets.{name}] must be an empty declaration"
        );
    }
}

#[test]
fn init_sqlite_declares_a_valid_database_and_empty_migrations_directory() {
    let root = tempfile::tempdir().unwrap();
    let r = pass(root.path(), &["init", "--engine", "sqlite"]);
    assert_eq!(r["command"], "init");
    assert_eq!(r["database"], "app");
    assert_eq!(r["engine"], "sqlite");
    assert_eq!(r["manifest"]["action"], "created");
    assert_eq!(r["migrations_dir"]["action"], "created");
    assert_eq!(r["secrets"], serde_json::json!(SQLITE_SECRETS));
    assert!(!r["next_steps"].as_array().unwrap().is_empty());

    let toml = manifest(root.path());
    let app = &toml["databases"]["app"];
    assert_eq!(app["engine"].as_str(), Some("sqlite"));
    assert_eq!(app["schema"].as_str(), Some("main"));
    assert_eq!(app["migrations_dir"].as_str(), Some("migrations"));
    assert_eq!(
        app["approval_secret"].as_str(),
        Some("APP_MIGRATION_APPROVAL")
    );
    assert_eq!(
        app["targets"]["development"]["connection_secret"].as_str(),
        Some("APP_DATABASE_FILE")
    );
    assert_eq!(
        app["scratch"]["connection_secret"].as_str(),
        Some("APP_REPLAY_FILE")
    );
    assert_eq!(app["scratch"]["disposable"].as_bool(), Some(true));
    let text = std::fs::read_to_string(root.path().join("ntnt.toml")).unwrap();
    assert_names_only(&text, &SQLITE_SECRETS);
    assert!(text.contains("# "), "generated section is commented");
    assert!(root.path().join("migrations").is_dir());
    assert_eq!(
        std::fs::read_dir(root.path().join("migrations"))
            .unwrap()
            .count(),
        0,
        "init writes no migration file"
    );

    // The manifest passes the full db configuration check and stops only at
    // the unset target secret, before any file or server is touched.
    assert_eq!(status_code(root.path(), &[]), "DB_SECRET");
}

#[test]
fn init_postgresql_declares_the_documented_target_scratch_and_secrets() {
    let root = tempfile::tempdir().unwrap();
    let r = pass(root.path(), &["init", "--engine", "postgresql"]);
    assert_eq!(r["engine"], "postgresql");
    assert_eq!(r["secrets"], serde_json::json!(POSTGRES_SECRETS));

    let toml = manifest(root.path());
    let app = &toml["databases"]["app"];
    assert_eq!(app["engine"].as_str(), Some("postgresql"));
    assert_eq!(app["schema"].as_str(), Some("app"));
    assert_eq!(app["migrations_dir"].as_str(), Some("migrations"));
    assert_eq!(
        app["approval_secret"].as_str(),
        Some("APP_MIGRATION_APPROVAL")
    );
    let target = &app["targets"]["development"];
    assert_eq!(target["connection_secret"].as_str(), Some("APP_DATABASE"));
    assert_eq!(target["database"].as_str(), Some("app_development"));
    assert_eq!(target["role"].as_str(), Some("app_owner"));
    let scratch = &app["scratch"];
    assert_eq!(
        scratch["connection_secret"].as_str(),
        Some("APP_REPLAY_DATABASE")
    );
    assert_eq!(scratch["database"].as_str(), Some("app_replay"));
    assert_eq!(scratch["role"].as_str(), Some("app_replay_owner"));
    assert_eq!(scratch["disposable"].as_bool(), Some(true));
    let text = std::fs::read_to_string(root.path().join("ntnt.toml")).unwrap();
    assert_names_only(&text, &POSTGRES_SECRETS);
    assert!(root.path().join("migrations").is_dir());
    assert_eq!(status_code(root.path(), &[]), "DB_SECRET");
}

#[test]
fn init_writes_the_documented_quick_start_manifests() {
    let guide = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/DATABASE_MIGRATIONS.md"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    let quick_start = guide
        .split("\n## Quick start\n")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    for (heading, engine) in [
        ("\n### SQLite\n", "sqlite"),
        ("\n### PostgreSQL\n", "postgresql"),
    ] {
        let documented: toml::Value = quick_start
            .split(heading)
            .nth(1)
            .unwrap()
            .split("```toml\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        pass(root.path(), &["init", "--engine", engine]);
        assert_eq!(
            manifest(root.path()),
            documented,
            "the {engine} quick start manifest differs from `ntnt db init`"
        );
    }
}

#[test]
fn init_human_output_lists_secret_names_and_next_steps_without_values() {
    let root = tempfile::tempdir().unwrap();
    let output = db(root.path())
        .args(["init", "--engine", "postgresql", "--database", "shop"])
        .bounded_output();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "SHOP_DATABASE",
        "SHOP_REPLAY_DATABASE",
        "SHOP_MIGRATION_APPROVAL",
        "shop_development",
        "ntnt db new",
        "ntnt db plan --target development",
        "docs/DATABASE_MIGRATIONS.md",
    ] {
        assert!(stdout.contains(expected), "{expected:?} missing:\n{stdout}");
    }
    assert!(!stdout.to_ascii_lowercase().contains("password="));
    let toml = manifest(root.path());
    assert_eq!(
        toml["databases"]["shop"]["targets"]["development"]["role"].as_str(),
        Some("shop_owner")
    );
    assert_eq!(
        status_code(root.path(), &["--database", "shop"]),
        "DB_SECRET"
    );
}

#[test]
fn init_appends_to_an_existing_manifest_and_keeps_its_bytes_as_a_prefix() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ntnt.toml");
    // Unrelated application configuration, ending without a newline.
    let original = b"# application settings\n[lint]\nstrict = true\n\n[secrets.SESSION_KEY]\nrequired = true # keep this comment".to_vec();
    std::fs::write(&path, &original).unwrap();
    let r = pass(root.path(), &["init", "--engine", "sqlite"]);
    assert_eq!(r["manifest"]["action"], "appended");
    let updated = std::fs::read(&path).unwrap();
    assert!(
        updated.starts_with(&original),
        "existing bytes must be an exact prefix:\n{}",
        String::from_utf8_lossy(&updated)
    );
    let toml = manifest(root.path());
    assert_eq!(toml["lint"]["strict"].as_bool(), Some(true));
    assert_eq!(
        toml["secrets"]["SESSION_KEY"]["required"].as_bool(),
        Some(true)
    );
    assert_eq!(toml["databases"]["app"]["engine"].as_str(), Some("sqlite"));
    assert_names_only(
        std::str::from_utf8(&updated[original.len()..]).unwrap(),
        &[],
    );
    assert_eq!(status_code(root.path(), &[]), "DB_SECRET");
}

#[test]
fn init_refuses_declared_databases_and_secrets_without_changing_anything() {
    let declared_database = r#"
[databases.app]
engine = "postgresql"
schema = "app"
migrations_dir = "db"
[databases.app.targets.production]
connection_secret = "PRODUCTION_DATABASE"
database = "app"
role = "app"
[secrets.PRODUCTION_DATABASE]
"#;
    for (existing, engine, declaration) in [
        (declared_database, "sqlite", "[databases.app]"),
        (
            "[secrets.APP_MIGRATION_APPROVAL]\n",
            "sqlite",
            "[secrets.APP_MIGRATION_APPROVAL]",
        ),
        (
            "[secrets.APP_REPLAY_FILE]\nrequired = true\n",
            "sqlite",
            "[secrets.APP_REPLAY_FILE]",
        ),
        (
            "[secrets.APP_DATABASE]\n",
            "postgresql",
            "[secrets.APP_DATABASE]",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ntnt.toml"), existing).unwrap();
        let before = snapshot(root.path());
        let r = refused(
            root.path(),
            &["init", "--engine", engine],
            "DB_INIT_CONFLICT",
        );
        let diagnostic = &r["diagnostics"][0];
        assert!(
            diagnostic["message"]
                .as_str()
                .unwrap()
                .contains(declaration),
            "the refusal names the existing declaration: {diagnostic}"
        );
        assert!(diagnostic["next_action"]
            .as_str()
            .unwrap()
            .contains("--database"));
        assert_eq!(snapshot(root.path()), before, "refusal changed files");
        assert!(!root.path().join("migrations").exists());
    }
}

#[test]
fn init_twice_refuses_the_second_run_and_keeps_the_first_manifest() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    let before = snapshot(root.path());
    refused(
        root.path(),
        &["init", "--engine", "postgresql"],
        "DB_INIT_CONFLICT",
    );
    assert_eq!(snapshot(root.path()), before);
}

/// init holds an exclusive lock on an existing manifest while extending it. While
/// another process holds that lock, init refuses and writes nothing.
#[test]
fn init_refuses_a_manifest_another_init_is_updating() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ntnt.toml");
    std::fs::write(&path, "[lint]\nstrict = true\n").unwrap();
    let before = snapshot(root.path());
    let holder = std::fs::File::open(&path).unwrap();
    holder.try_lock().unwrap();
    let r = refused(
        root.path(),
        &["init", "--engine", "sqlite"],
        "DB_INIT_CONFLICT",
    );
    let message = r["diagnostics"][0]["message"].as_str().unwrap();
    assert!(message.contains("Another `ntnt db init`"), "{message}");
    // Windows locks are mandatory: read the files once the lock is released.
    drop(holder);
    assert_eq!(
        snapshot(root.path()),
        before,
        "a refused init changed files"
    );
    pass(root.path(), &["init", "--engine", "sqlite"]);
}

/// A manifest the user may not write is refused as an I/O failure whose next action
/// asks for a writable ntnt.toml, and init writes nothing.
#[test]
fn init_refuses_a_manifest_it_cannot_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("ntnt.toml");
    std::fs::write(&path, "[lint]\nstrict = true\n").unwrap();
    let writable = std::fs::metadata(&path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&path, read_only).unwrap();
    // A process that may write it anyway (root on Unix) cannot observe the refusal.
    if std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .is_err()
    {
        let before = snapshot(root.path());
        let r = refused(
            root.path(),
            &["init", "--engine", "sqlite"],
            "DB_SCAFFOLD_IO",
        );
        let diagnostic = &r["diagnostics"][0];
        assert!(
            diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("not writable"),
            "{r}"
        );
        let message = diagnostic["message"].as_str().unwrap();
        assert!(
            message.contains("ntnt.toml") && message.contains("(os error"),
            "{r}"
        );
        assert!(
            diagnostic["next_action"]
                .as_str()
                .unwrap()
                .contains("writable regular file"),
            "{r}"
        );
        assert_eq!(
            snapshot(root.path()),
            before,
            "a refused init changed files"
        );
        assert!(!root.path().join("migrations").exists());
    }
    std::fs::set_permissions(&path, writable).unwrap();
}

#[test]
fn init_never_replaces_a_non_directory_migrations_path_or_shares_a_directory() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("migrations"), "not a directory").unwrap();
    let before = snapshot(root.path());
    refused(
        root.path(),
        &["init", "--engine", "sqlite"],
        "DB_INIT_CONFLICT",
    );
    assert_eq!(snapshot(root.path()), before);
    assert!(!root.path().join("ntnt.toml").exists());

    // A second logical database must not read the first one's migration files.
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    let before = snapshot(root.path());
    refused(
        root.path(),
        &["init", "--engine", "sqlite", "--database", "analytics"],
        "DB_INIT_CONFLICT",
    );
    assert_eq!(snapshot(root.path()), before);
}

#[test]
fn init_keeps_an_existing_migrations_directory_and_its_files() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("migrations")).unwrap();
    std::fs::write(
        root.path().join("migrations").join("0001_initial.sql"),
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY);\n",
    )
    .unwrap();
    let before = snapshot(&root.path().join("migrations"));
    let r = pass(root.path(), &["init", "--engine", "sqlite"]);
    assert_eq!(r["migrations_dir"]["action"], "existing");
    assert_eq!(snapshot(&root.path().join("migrations")), before);
}

#[test]
fn init_rejects_database_names_that_cannot_name_secrets_and_roles() {
    let long = "a".repeat(51);
    for name in [
        "",
        "App",
        "my-app",
        "1app",
        "_app",
        "app name",
        "pg_app",
        "information_schema",
        long.as_str(),
    ] {
        let root = tempfile::tempdir().unwrap();
        refused(
            root.path(),
            &["init", "--engine", "postgresql", "--database", name],
            "DB_INIT_NAME",
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    let root = tempfile::tempdir().unwrap();
    pass(
        root.path(),
        &[
            "init",
            "--engine",
            "postgresql",
            "--database",
            &"a".repeat(50),
        ],
    );
}

#[test]
fn new_creates_sequential_migration_files_without_secrets_or_a_connection() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    let migrations = root.path().join("migrations");
    let first = pass(root.path(), &["new", "initial"]);
    assert_eq!(first["command"], "new");
    assert_eq!(first["database"], "app");
    assert_eq!(first["migration"]["id"], "1");
    assert_eq!(first["migration"]["source"], "0001_initial.sql");
    let created = std::fs::read_to_string(migrations.join("0001_initial.sql")).unwrap();
    assert!(created.starts_with("-- "), "{created}");
    assert!(created.lines().all(|l| l.is_empty() || l.starts_with("--")));
    let before = snapshot(&migrations);
    let second = pass(root.path(), &["new", "add_accounts"]);
    assert_eq!(second["migration"]["id"], "2");
    assert_eq!(second["migration"]["source"], "0002_add_accounts.sql");
    assert!(migrations.join("0002_add_accounts.sql").is_file());
    let after = snapshot(&migrations);
    assert_eq!(after.len(), 2);
    assert_eq!(after["0001_initial.sql"], before["0001_initial.sql"]);

    let output = db(root.path()).args(["new", "add_index"]).bounded_output();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("0003_add_index.sql"));
}

#[test]
fn new_numbers_after_the_highest_existing_id_and_keeps_its_padding() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "postgresql"]);
    let migrations = root.path().join("migrations");
    for (name, sql) in [
        (
            "0003_accounts.sql",
            "CREATE TABLE app.accounts (id BIGINT);\n",
        ),
        ("0003_accounts.toml", "intent_ids = [\"accounts\"]\n"),
        ("0010_x.sql", "CREATE TABLE app.x (id BIGINT);\n"),
        ("2_early.sql", "CREATE TABLE app.early (id BIGINT);\n"),
    ] {
        std::fs::write(migrations.join(name), sql).unwrap();
    }
    let before = snapshot(&migrations);
    let r = pass(root.path(), &["new", "next"]);
    assert_eq!(r["migration"]["source"], "0011_next.sql");
    let after = snapshot(&migrations);
    for (name, bytes) in &before {
        assert_eq!(&after[name], bytes, "{name} changed");
    }
    assert_eq!(after.len(), before.len() + 1);

    // Wider existing IDs keep their width; numbering is numeric, not lexical.
    std::fs::write(migrations.join("000120_wide.sql"), "SELECT 1;\n").unwrap();
    let r = pass(root.path(), &["new", "wider"]);
    assert_eq!(r["migration"]["source"], "000121_wider.sql");
}

#[test]
fn new_rejects_invalid_names_without_creating_files() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    let long = "a".repeat(65);
    for name in [
        "",
        "AddAccounts",
        "add-accounts",
        "add accounts",
        "../escape",
        "x.sql",
        "x/y",
        long.as_str(),
    ] {
        refused(root.path(), &["new", name], "DB_MIGRATION_NAME");
    }
    assert_eq!(
        std::fs::read_dir(root.path().join("migrations"))
            .unwrap()
            .count(),
        0
    );
    pass(root.path(), &["new", &"a".repeat(64)]);
}

#[test]
fn new_applies_the_database_selection_rule() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("app-migrations")).unwrap();
    std::fs::create_dir(root.path().join("analytics-migrations")).unwrap();
    std::fs::write(
        root.path().join("ntnt.toml"),
        r#"
[databases.app]
engine = "sqlite"
schema = "main"
migrations_dir = "app-migrations"
[databases.app.targets.development]
connection_secret = "APP_DATABASE_FILE"

[databases.analytics]
engine = "postgresql"
schema = "analytics"
migrations_dir = "analytics-migrations"
[databases.analytics.targets.development]
connection_secret = "ANALYTICS_DATABASE"
database = "analytics_development"
role = "analytics_owner"

[secrets.APP_DATABASE_FILE]
[secrets.ANALYTICS_DATABASE]
"#,
    )
    .unwrap();
    let before = snapshot(root.path());
    refused(root.path(), &["new", "events"], "DB_DATABASE_REQUIRED");
    refused(
        root.path(),
        &["new", "events", "--database", "missing"],
        "DB_DATABASE_UNKNOWN",
    );
    assert_eq!(snapshot(root.path()), before);
    let r = pass(root.path(), &["new", "events", "--database", "analytics"]);
    assert_eq!(r["database"], "analytics");
    assert!(root
        .path()
        .join("analytics-migrations")
        .join("0001_events.sql")
        .is_file());
    assert_eq!(
        std::fs::read_dir(root.path().join("app-migrations"))
            .unwrap()
            .count(),
        0
    );
}

/// The plan command `new` prints keeps a custom manifest and an explicitly selected
/// database; followed as printed, it gets past manifest and database selection.
#[test]
fn new_prints_a_plan_command_for_the_selected_manifest_and_database() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("app-migrations")).unwrap();
    std::fs::create_dir(config.join("analytics-migrations")).unwrap();
    std::fs::write(
        config.join("database.toml"),
        r#"
[databases.app]
engine = "sqlite"
schema = "main"
migrations_dir = "app-migrations"
[databases.app.targets.development]
connection_secret = "APP_DATABASE_FILE"

[databases.analytics]
engine = "sqlite"
schema = "main"
migrations_dir = "analytics-migrations"
[databases.analytics.targets.development]
connection_secret = "ANALYTICS_DATABASE_FILE"

[secrets.APP_DATABASE_FILE]
[secrets.ANALYTICS_DATABASE_FILE]
"#,
    )
    .unwrap();
    let output = db(root.path())
        .args(["new", "events", "--manifest", "config/database.toml"])
        .args(["--database", "analytics"])
        .bounded_output();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success(), "{stdout}");
    assert!(config
        .join("analytics-migrations/0001_events.sql")
        .is_file());
    let printed = commands(&stdout);
    assert_eq!(printed.len(), 1, "{stdout}");
    let command = printed[0].replace("<target>", "development");
    let words: Vec<&str> = command.split_whitespace().collect();
    assert_eq!(words[..3], ["ntnt", "db", "plan"], "{command}");
    let output = db(root.path())
        .args(&words[2..])
        .arg("--json")
        .env_remove("ANALYTICS_DATABASE_FILE")
        .bounded_output();
    assert!(!output.status.success());
    // Selection succeeded: only the unset target secret stops the command.
    assert_eq!(
        report(&output)["diagnostics"][0]["code"],
        "DB_SECRET",
        "{command}"
    );
}

/// Every command init prints for a manifest path that needs shell quoting runs as
/// printed in a POSIX shell, through migrate, and `new` prints the same plan command.
#[cfg(unix)]
#[test]
fn printed_commands_run_as_printed_for_a_manifest_path_that_needs_quoting() {
    let root = tempfile::tempdir().unwrap();
    let project = "team's project $HOME";
    std::fs::create_dir(root.path().join(project)).unwrap();
    let r = pass(
        root.path(),
        &[
            "init",
            "--engine",
            "sqlite",
            "--manifest",
            &format!("{project}/ntnt.toml"),
        ],
    );
    let steps: Vec<&str> = r["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step.as_str().unwrap())
        .collect();
    let printed = commands(&steps.join("\n"));
    let [new, status, plan, migrate] = printed.as_slice() else {
        panic!("{printed:?}");
    };
    // Empty files are valid SQLite databases. Use native paths (macOS /private).
    let native = root.path().canonicalize().unwrap();
    let target = native.join("app.sqlite");
    let replay = native.join("replay.sqlite");
    std::fs::File::create(&target).unwrap();
    std::fs::File::create(&replay).unwrap();
    // `ntnt` in a printed command resolves to the binary under test.
    let binaries = Path::new(env!("CARGO_BIN_EXE_ntnt")).parent().unwrap();
    let search = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(binaries.to_path_buf()).chain(std::env::split_paths(&search)),
    )
    .unwrap();
    let shell = |command: &str, approval: Option<&str>| {
        let mut sh = Command::new("sh");
        // exec: the bounded child is ntnt itself, never an orphaned grandchild.
        sh.arg("-c")
            .arg(format!("exec {command}"))
            .current_dir(root.path())
            .env("PATH", &path)
            .env("NTNT_ENV", "development")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .env("APP_DATABASE_FILE", &target)
            .env("APP_REPLAY_FILE", &replay)
            .env_remove("APP_MIGRATION_APPROVAL");
        if let Some(digest) = approval {
            sh.env("APP_MIGRATION_APPROVAL", digest);
        }
        let output = sh.bounded_output();
        assert!(
            output.status.success(),
            "{command}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let output = shell(new, None);
    assert_eq!(
        commands(&String::from_utf8_lossy(&output.stdout)),
        [plan.replace("--target development", "--target <target>")],
        "new and init print the same plan command"
    );
    let migration = root
        .path()
        .join(project)
        .join("migrations")
        .join("0001_initial.sql");
    let mut sql = std::fs::read_to_string(&migration).unwrap();
    sql.push_str("CREATE TABLE accounts (id INTEGER PRIMARY KEY);\n");
    std::fs::write(&migration, sql).unwrap();
    let json = |output: Output| -> Value { report(&output) };
    assert_eq!(
        json(shell(&format!("{status} --json"), None))["outcome"],
        "pass"
    );
    let reviewed = json(shell(&format!("{plan} --json"), None));
    let digest = reviewed["plan_digest"].as_str().unwrap();
    let applied = json(shell(&format!("{migrate} --json"), Some(digest)));
    assert_eq!(applied["progress"][0]["state"], "committed", "{applied}");
}

/// `ntnt db new` takes the manifest lock init takes and holds it from listing the
/// migrations until the new file exists. While another process holds it, `new`
/// refuses at once with exit status 1, names the manifest and creates nothing.
#[test]
fn new_refuses_while_another_process_holds_the_manifest_lock() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    let before = snapshot(root.path());
    // Opened for writing, as `new` opens it: NFS locks only such a handle.
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("ntnt.toml"))
        .unwrap();
    holder.try_lock().unwrap();
    let json = db(root.path())
        .args(["new", "accounts", "--json"])
        .bounded_output();
    let text = db(root.path()).args(["new", "accounts"]).bounded_output();
    // Windows locks are mandatory: read the files once the lock is released.
    drop(holder);
    let r = report(&json);
    assert_eq!(json.status.code(), Some(1), "{r}");
    assert_eq!(r["outcome"], "fail", "{r}");
    let diagnostic = &r["diagnostics"][0];
    assert_eq!(diagnostic["code"], "DB_NEW_CONFLICT", "{r}");
    assert_eq!(
        diagnostic["message"],
        "Another `ntnt db new` or `ntnt db init` holds the lock on the manifest ntnt.toml; no migration was created.",
        "{r}"
    );
    let next = diagnostic["next_action"].as_str().unwrap();
    assert!(next.contains("Rerun `ntnt db new`"), "{next}");
    let stdout = String::from_utf8(text.stdout).unwrap();
    assert_eq!(text.status.code(), Some(1), "{stdout}");
    assert_eq!(
        stdout,
        format!(
            "Database new: fail\nDB_NEW_CONFLICT: {}\n  Next: {next}\n",
            diagnostic["message"].as_str().unwrap()
        )
    );
    assert_eq!(snapshot(root.path()), before, "a refused new changed files");
    let r = pass(root.path(), &["new", "accounts"]);
    assert_eq!(r["migration"]["source"], "0001_accounts.sql");
}

/// A manifest `new` may not write is locked through a read-only handle, which local
/// filesystems accept: `new` still refuses while another process holds the lock,
/// and otherwise numbers the migration as before and leaves the manifest as it was.
#[test]
fn new_locks_a_read_only_manifest() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    let path = root.path().join("ntnt.toml");
    let writable = std::fs::metadata(&path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&path, read_only).unwrap();
    let before = std::fs::read(&path).unwrap();
    // Opened the way `new` opens it (input_file::open_replaceable): for writing
    // when the process may (root may, despite the mode), read-only otherwise.
    // The lock the holder gets is then the one `new` would get.
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .or_else(|_| std::fs::File::open(&path))
        .unwrap();
    match holder.try_lock() {
        Ok(()) => {}
        // NFS refuses an exclusive lock on a read-only handle. `new` cannot
        // lock this manifest either: it stops with DB_SCAFFOLD_IO, creating
        // nothing.
        Err(std::fs::TryLockError::Error(_)) => {
            drop(holder);
            refused(root.path(), &["new", "accounts"], "DB_SCAFFOLD_IO");
            assert_eq!(
                std::fs::read_dir(root.path().join("migrations"))
                    .unwrap()
                    .count(),
                0
            );
            std::fs::set_permissions(&path, writable).unwrap();
            return;
        }
        Err(std::fs::TryLockError::WouldBlock) => panic!("nothing else holds the manifest lock"),
    }
    refused(root.path(), &["new", "accounts"], "DB_NEW_CONFLICT");
    drop(holder);
    assert_eq!(
        std::fs::read_dir(root.path().join("migrations"))
            .unwrap()
            .count(),
        0
    );
    let r = pass(root.path(), &["new", "accounts"]);
    assert_eq!(r["migration"]["source"], "0001_accounts.sql");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
    std::fs::set_permissions(&path, writable).unwrap();
}

#[test]
fn new_requires_a_manifest_and_an_existing_migrations_directory() {
    let root = tempfile::tempdir().unwrap();
    refused(root.path(), &["new", "initial"], "DB_INPUT_IO");
    pass(root.path(), &["init", "--engine", "sqlite"]);
    std::fs::remove_dir(root.path().join("migrations")).unwrap();
    refused(root.path(), &["new", "initial"], "DB_CONFIG");
    assert!(!root.path().join("migrations").exists());
}

#[test]
fn sqlite_scaffold_runs_the_documented_plan_approve_migrate_loop() {
    let root = tempfile::tempdir().unwrap();
    pass(root.path(), &["init", "--engine", "sqlite"]);
    pass(root.path(), &["new", "initial"]);
    let migration = root.path().join("migrations").join("0001_initial.sql");
    let mut sql = std::fs::read_to_string(&migration).unwrap();
    sql.push_str("CREATE TABLE accounts (\n    id INTEGER PRIMARY KEY,\n    email TEXT NOT NULL UNIQUE\n);\n");
    std::fs::write(&migration, sql).unwrap();
    // Empty files are valid SQLite databases. Use native paths (macOS /private).
    let native = root.path().canonicalize().unwrap();
    let target = native.join("app.sqlite");
    let replay = native.join("replay.sqlite");
    std::fs::File::create(&target).unwrap();
    std::fs::File::create(&replay).unwrap();
    let run = |args: &[&str], approval: Option<&str>| {
        let mut command = db(root.path());
        command
            .args(args)
            .args(["--target", "development", "--json"])
            .env("APP_DATABASE_FILE", &target)
            .env("APP_REPLAY_FILE", &replay);
        if let Some(digest) = approval {
            command.env("APP_MIGRATION_APPROVAL", digest);
        }
        let output = command.bounded_output();
        let r = report(&output);
        assert_eq!(r["outcome"], "pass", "{r}");
        assert!(output.status.success());
        r
    };
    run(&["status"], None);
    let plan = run(&["plan", "--out", "review.json"], None);
    let digest = plan["plan_digest"].as_str().unwrap();
    let applied = run(&["migrate", "--plan", "review.json"], Some(digest));
    assert_eq!(applied["progress"][0]["state"], "committed");
    run(&["verify"], None);
    let next = pass(root.path(), &["new", "add_timezone"]);
    assert_eq!(next["migration"]["source"], "0002_add_timezone.sql");
}

/// init and new run outside the db commands' runtime deadline, so every child in
/// this file is bounded: a stalled `ntnt` process fails its test at the deadline
/// (killed and reaped) instead of hanging the suite.
#[test]
fn a_stalled_cli_child_is_stopped_at_its_deadline() {
    let root = tempfile::tempdir().unwrap();
    // An hour: the child cannot exit on its own while this test runs, so only
    // the deadline under test can end it, however slow the machine is.
    std::fs::write(
        root.path().join("stall.tnt"),
        "import { sleep } from \"std/time\"\nsleep(3600000)\n",
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command
        .args(["run", "stall.tnt"])
        .env_remove("NTNT_ENV")
        .current_dir(root.path());
    // output_within returns only after killing and reaping the child. Without the
    // deadline or the kill it would wait an hour, so the hang guard every child in
    // this file gets (`DEADLINE`) fails the test instead.
    let (done, returned) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(bounded_output::output_within(
            &mut command,
            Duration::from_secs(1),
        ));
    });
    let result = returned
        .recv_timeout(bounded_output::DEADLINE)
        .expect("output_within did not stop the stalled child at its deadline");
    let stalled = result.expect_err("a child past its deadline fails the test");
    assert!(
        stalled.to_string().contains("did not exit within"),
        "{stalled}"
    );
    // The status is what wait returned once the child was reaped: evidence of the
    // kill, not text about it. On Unix only SIGKILL ends the hour-long sleep.
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&stalled.status),
        Some(libc::SIGKILL),
        "{stalled}"
    );
    assert!(!stalled.status.success(), "{stalled}");
}
