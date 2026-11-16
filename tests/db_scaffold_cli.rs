//! `ntnt db init` and `ntnt db new` scaffold first-time database setup through
//! the built CLI. They declare names, never secret values, and never replace files.
#[path = "support/bounded_output.rs"]
mod bounded_output;
use bounded_output::BoundedOutput;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

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
    std::fs::write(
        root.path().join("stall.tnt"),
        "import { sleep } from \"std/time\"\nsleep(20000)\n",
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command
        .args(["run", "stall.tnt"])
        .env_remove("NTNT_ENV")
        .current_dir(root.path());
    let started = Instant::now();
    let result = bounded_output::output_within(&mut command, Duration::from_secs(1));
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(10),
        "the stalled child was not stopped at its deadline ({elapsed:?})"
    );
    let failure = result.expect_err("a child past its deadline fails the test");
    assert!(failure.contains("did not exit within"), "{failure}");
}
