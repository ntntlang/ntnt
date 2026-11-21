//! The same lifecycle, contract and recovery expectations run on both adapters.
#[path = "support/db_fixture.rs"]
mod pg_fixture;
#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use serde_json::Value;
use std::path::Path;

enum Fixture {
    Postgres(Box<pg_fixture::Fixture>),
    Sqlite(sqlite_fixture::Fixture),
}
impl Fixture {
    fn pg() -> Self {
        Self::Postgres(Box::new(pg_fixture::Fixture::new()))
    }
    fn sqlite() -> Self {
        Self::Sqlite(sqlite_fixture::Fixture::new())
    }
    fn schema(&self) -> &str {
        match self {
            Self::Postgres(_) => "app",
            Self::Sqlite(_) => "main",
        }
    }
    fn root(&self) -> &Path {
        match self {
            Self::Postgres(f) => f.root.path(),
            Self::Sqlite(f) => f.root.path(),
        }
    }
    fn sql(&self, name: &str, sql: &str) {
        match self {
            Self::Postgres(f) => f.sql(name, sql),
            Self::Sqlite(f) => f.sql(name, sql),
        }
    }
    fn command(&self, command: &str, args: &[&str]) -> Value {
        match self {
            Self::Postgres(f) => f.command_args(command, args),
            Self::Sqlite(f) => f.command(command, args),
        }
    }
    fn plan(&self, path: &str) -> Value {
        match self {
            Self::Postgres(f) => f.plan(path),
            Self::Sqlite(f) => f.plan(path),
        }
    }
    fn apply(&self, path: &str, approval: Option<&str>) -> Value {
        match self {
            Self::Postgres(f) => f.apply(path, approval),
            Self::Sqlite(f) => f.apply(path, approval),
        }
    }
    fn execute(&self, sql: &str) {
        match self {
            Self::Postgres(f) => f.target().batch_execute(sql).unwrap(),
            Self::Sqlite(f) => f.target().execute_batch(sql).unwrap(),
        }
    }
    fn text(&self, sql: &str) -> String {
        match self {
            Self::Postgres(f) => f.target().query_one(sql, &[]).unwrap().get(0),
            Self::Sqlite(f) => f.target().query_row(sql, [], |r| r.get(0)).unwrap(),
        }
    }
    fn count(&self, sql: &str) -> i64 {
        match self {
            Self::Postgres(f) => f.target().query_one(sql, &[]).unwrap().get(0),
            Self::Sqlite(f) => f.target().query_row(sql, [], |r| r.get(0)).unwrap(),
        }
    }
    fn ledger(&self) -> &str {
        match self {
            Self::Postgres(_) => "_ntnt.schema_migrations_v2",
            Self::Sqlite(_) => "main._ntnt_schema_migrations_v2",
        }
    }
    /// A command's JSON report for `target` instead of the fixture's own,
    /// with `approval` as the approval secret.
    fn run(&self, command: &str, target: &str, args: &[&str], approval: Option<&str>) -> Value {
        let template = match self {
            Self::Postgres(f) => f.process(command),
            Self::Sqlite(f) => f.process(command),
        };
        let mut process = std::process::Command::new(template.get_program());
        let mut previous = None;
        for arg in template.get_args() {
            process.arg(if previous == Some("--target") {
                std::ffi::OsStr::new(target)
            } else {
                arg
            });
            previous = arg.to_str();
        }
        for (key, value) in template.get_envs() {
            match value {
                Some(value) => process.env(key, value),
                None => process.env_remove(key),
            };
        }
        if let Some(approval) = approval {
            process.env("DB_APPROVAL", approval);
        }
        let output = process
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    }
    /// Declare a second target, `staging`, whose secret is never set.
    fn declare_staging(&self) {
        let endpoint = match self {
            Self::Postgres(_) => {
                "database = \"ntnt_staging_unused\"\nrole = \"ntnt_staging_unused\"\n"
            }
            Self::Sqlite(_) => "",
        };
        let manifest = self.root().join("ntnt.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(
            &manifest,
            format!("{text}\n[databases.app.targets.staging]\nconnection_secret = \"DB_STAGING\"\n{endpoint}[secrets.DB_STAGING]\n"),
        )
        .unwrap();
    }
    /// A command's human (non-JSON) output.
    fn human(&self, command: &str, args: &[&str]) -> String {
        let template = match self {
            Self::Postgres(f) => f.process(command),
            Self::Sqlite(f) => f.process(command),
        };
        let mut process = std::process::Command::new(template.get_program());
        process.args(
            template
                .get_args()
                .filter(|arg| *arg != std::ffi::OsStr::new("--json")),
        );
        for (key, value) in template.get_envs() {
            match value {
                Some(value) => process.env(key, value),
                None => process.env_remove(key),
            };
        }
        let output = process
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    }
}
/// The diagnostic with `code` in `report`.
fn diagnostic<'a>(report: &'a Value, code: &str) -> &'a Value {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {report}"))
}
fn code(report: &Value, code: &str) -> bool {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}
fn first(f: &Fixture) -> Value {
    f.sql(
        "0001_initial.sql",
        &format!(
            "CREATE TABLE {}.items(id INTEGER PRIMARY KEY,value TEXT);",
            f.schema()
        ),
    );
    f.plan("initial.json")
}
fn prefix_recovery(f: Fixture) {
    let initial = first(&f);
    for approval in [None, Some("not-approved")] {
        assert!(code(
            &f.apply("initial.json", approval),
            "DB_APPROVAL_REQUIRED"
        ));
    }
    assert_eq!(f.command("status", &[])["history"]["applied_count"], 0);
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.execute(&format!(
        "INSERT INTO {}.items VALUES(17,'retained'),(-1,'remove-before-retry')",
        f.schema()
    ));
    f.sql(
        "0002_expand.sql",
        &format!(
            "ALTER TABLE {}.items ADD COLUMN label TEXT DEFAULT 'mapped';",
            f.schema()
        ),
    );
    f.sql(
        "0002_expand.toml",
        &format!(
            "[requires]\nrows='SELECT count(*) = 2 FROM {}.items'",
            f.schema()
        ),
    );
    f.sql(
        "0003_finish.sql",
        &format!(
            "ALTER TABLE {}.items ADD COLUMN active INTEGER DEFAULT 1;",
            f.schema()
        ),
    );
    f.sql(
        "0003_finish.toml",
        &format!(
            "[ensures]\npositive='SELECT NOT EXISTS (SELECT 1 FROM {}.items WHERE id < 0)'",
            f.schema()
        ),
    );
    let plan = f.plan("upgrade.json");
    let failed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_CHECK_FAILED"), "{failed}");
    assert_eq!(failed["progress"][1]["state"], "committed");
    assert_eq!(failed["progress"][2]["state"], "failed");
    assert_eq!(failed["assertions"]["executed"], 2);
    assert_eq!(
        f.text(&format!(
            "SELECT value FROM {}.items WHERE id=17",
            f.schema()
        )),
        "retained"
    );
    f.execute(&format!("DELETE FROM {}.items WHERE id=-1", f.schema()));
    let resumed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(resumed["outcome"], "pass", "{resumed}");
    assert_eq!(resumed["progress"][1]["state"], "already_applied");
    assert_eq!(resumed["progress"][2]["state"], "committed");
    assert_eq!(resumed["assertions"]["executed"], 1);
    assert_eq!(f.count(&format!("SELECT count(*) FROM {}", f.ledger())), 3);
    f.execute(&format!(
        "INSERT INTO {}.items(id,value) VALUES(-1,'old ensures no longer active')",
        f.schema()
    ));
    let verified = f.command("verify", &[]);
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["assertions"]["executed"], 0);
    assert_eq!(
        f.apply("upgrade.json", plan["plan_digest"].as_str())["assertions"]["executed"],
        0
    );
}
fn history_and_drift(f: Fixture) {
    let initial = first(&f);
    let sql = std::fs::read_to_string(f.root().join("migrations/0001_initial.sql")).unwrap();
    f.sql("0001_initial.sql", &(sql.clone() + " -- edited"));
    assert!(code(
        &f.apply("initial.json", initial["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    f.sql("0001_initial.sql", &sql);
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql("0001_initial.toml", "# changed metadata");
    assert!(code(&f.command("status", &[]), "DB_HISTORY_EDITED"));
    std::fs::remove_file(f.root().join("migrations/0001_initial.toml")).unwrap();
    std::fs::remove_file(f.root().join("migrations/0001_initial.sql")).unwrap();
    assert!(code(&f.command("status", &[]), "DB_HISTORY_MISSING"));
    f.sql("0001_initial.sql", &sql);
    f.sql(
        "0002_pending.sql",
        &format!("ALTER TABLE {}.items ADD COLUMN pending TEXT", f.schema()),
    );
    assert_eq!(f.command("status", &[])["drift"]["outcome"], "pass");
    f.execute(&format!(
        "ALTER TABLE {}.items ADD COLUMN outside TEXT",
        f.schema()
    ));
    assert!(code(&f.command("verify", &[]), "DB_DRIFT"));
    f.execute(&format!("UPDATE {} SET state='running'", f.ledger()));
    assert!(code(&f.command("status", &[]), "DB_HISTORY_DIRTY"));
}
/// #423 (sq-13, pg-07, cli-15): history refusals name the migration, show
/// checksums but never SQL, and following each one's advice restores a
/// history that passes.
fn history_refusals_name_the_migration(f: Fixture) {
    let s = f.schema();
    let a = format!("CREATE TABLE {s}.a(id INTEGER PRIMARY KEY);\n");
    let b = format!("CREATE TABLE {s}.b(id INTEGER PRIMARY KEY);\n");
    let c = format!("CREATE TABLE {s}.c(id INTEGER PRIMARY KEY);\n");
    f.sql("0010_a.sql", &a);
    f.sql("0020_b.sql", &b);
    f.sql("0030_c.sql", &c);
    let plan = f.plan("initial.json");
    let applied = f.apply("initial.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let passes = || {
        let r = f.command("status", &[]);
        assert_eq!(r["outcome"], "pass", "{r}");
        r
    };
    let no_sql = |d: &Value| assert!(!d.to_string().contains("CREATE TABLE"), "{d}");

    // An editor or git core.autocrlf rewrote one applied file's line endings.
    f.sql("0020_b.sql", &b.replace('\n', "\r\n"));
    let r = f.command("status", &[]);
    let d = diagnostic(&r, "DB_HISTORY_EDITED");
    assert_eq!(d["source"], "0020_b.sql", "{d}");
    let message = d["message"].as_str().unwrap();
    let applied_checksum = plan["source"]["migrations"][1]["checksum"]
        .as_str()
        .unwrap();
    let checksum_now = r["source"]["migrations"][1]["checksum"].as_str().unwrap();
    assert_ne!(applied_checksum, checksum_now);
    for shown in ["0020_b.sql", applied_checksum, checksum_now] {
        assert!(message.contains(shown), "{shown}: {d}");
    }
    no_sql(d);
    // Following the advice: the applied bytes are restored.
    f.sql("0020_b.sql", &b);
    passes();

    // An applied file was deleted.
    std::fs::remove_file(f.root().join("migrations/0030_c.sql")).unwrap();
    let r = f.command("status", &[]);
    let d = diagnostic(&r, "DB_HISTORY_MISSING");
    assert_eq!(d["source"], "0030_c.sql", "{d}");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("0030_c.sql is missing from migrations/"),
        "{d}"
    );
    no_sql(d);
    f.sql("0030_c.sql", &c);
    passes();

    // A teammate's branch merged a migration numbered below the last applied one.
    f.sql(
        "0015_merged.sql",
        &format!("CREATE TABLE {s}.merged(id INTEGER PRIMARY KEY);\n"),
    );
    let r = f.command("status", &[]);
    let d = diagnostic(&r, "DB_HISTORY_DIVERGENT");
    assert_eq!(d["source"], "0015_merged.sql", "{d}");
    let message = d["message"].as_str().unwrap();
    assert!(
        message.contains("position 2") && message.contains("0020_b.sql"),
        "{d}"
    );
    let next_action = d["next_action"].as_str().unwrap();
    assert!(
        next_action.contains("above 30") && next_action.contains("0031_merged.sql"),
        "{d}"
    );
    no_sql(d);
    // The human report says what is known about the history, never null counts.
    let human = f.human("status", &[]);
    assert!(human.contains("DB_HISTORY_DIVERGENT"), "{human}");
    let history = human
        .lines()
        .find(|line| line.starts_with("History: "))
        .unwrap_or_else(|| panic!("no History line: {human}"));
    assert!(!history.contains("null"), "{history}");
    // Following the advice: the merged migration is renumbered above 30.
    std::fs::rename(
        f.root().join("migrations/0015_merged.sql"),
        f.root().join("migrations/0031_merged.sql"),
    )
    .unwrap();
    assert_eq!(passes()["history"]["pending_count"], 1);
}
/// #423 (sq-12, cli-09): a stale plan names the binding that changed, for
/// migrate and for --against; a plan for another target is refused before
/// any I/O; a hand-edited or forged plan is invalid, not a changed target;
/// and following the advice applies the plan.
fn stale_plans_name_what_changed(f: Fixture) {
    let s = f.schema();
    f.declare_staging();
    let sql = format!("CREATE TABLE {s}.items(id INTEGER PRIMARY KEY);\n");
    f.sql("0001_initial.sql", &sql);
    let plan = f.plan("review.json");
    let digest = plan["plan_digest"].as_str();
    let migrate = |target: &str, file: &str| f.run("migrate", target, &["--plan", file], digest);
    let stale = |r: &Value| -> String {
        assert_ne!(r["outcome"], "pass", "{r}");
        diagnostic(r, "DB_PLAN_STALE")["message"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // Another target: refused before any I/O, though its secret is unset.
    let r = migrate("staging", "review.json");
    assert!(!code(&r, "DB_SECRET"), "{r}");
    let wrong = "review.json was planned for database app, target test; this run selected database app, target staging.";
    assert_eq!(stale(&r), wrong, "{r}");
    let next_action = diagnostic(&r, "DB_PLAN_STALE")["next_action"]
        .as_str()
        .unwrap();
    assert!(next_action.contains("--database app --target test"), "{r}");
    let r = f.run("verify", "staging", &["--against", "review.json"], None);
    assert_eq!(r["plan_validation"]["outcome"], "fail", "{r}");
    assert_eq!(stale(&r), wrong, "{r}");

    // Edited by hand, or with a forged digest: invalid, never another target.
    let saved: Value =
        serde_json::from_slice(&std::fs::read(f.root().join("review.json")).unwrap()).unwrap();
    let mut edited = saved.clone();
    edited["plan"]["binding"]["target_name"] = serde_json::json!("staging");
    let mut forged = saved.clone();
    forged["plan_digest"] = serde_json::json!("0".repeat(64));
    for (file, artifact) in [("edited.json", &edited), ("forged.json", &forged)] {
        std::fs::write(f.root().join(file), artifact.to_string()).unwrap();
        for target in ["staging", "test"] {
            let message = stale(&migrate(target, file));
            assert!(message.starts_with("Saved plan is invalid"), "{message}");
            assert!(
                !message.contains("staging") && !message.contains("planned for"),
                "{message}"
            );
        }
    }

    // A pending migration edited after the plan was made.
    let checksum = plan["plan"]["pending"][0]["checksum"].as_str().unwrap();
    f.sql("0001_initial.sql", &format!("{sql}-- reviewed later\n"));
    let message = stale(&migrate("test", "review.json"));
    assert!(
        message.starts_with("0001_initial.sql changed after the plan was made: checksum: plan "),
        "{message}"
    );
    assert!(message.contains(checksum), "{message}");
    f.sql("0001_initial.sql", &sql);
    // A migration added after the plan was made.
    f.sql(
        "0002_more.sql",
        &format!("CREATE TABLE {s}.more(id INTEGER);\n"),
    );
    assert_eq!(
        stale(&migrate("test", "review.json")),
        "0002_more.sql was added to migrations/ after the plan was made."
    );
    std::fs::remove_file(f.root().join("migrations/0002_more.sql")).unwrap();
    // The manifest's bytes, but not the database's configuration.
    let manifest = f.root().join("ntnt.toml");
    let declared = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{declared}# reviewed\n")).unwrap();
    let message = stale(&migrate("test", "review.json"));
    assert!(
        message.starts_with("ntnt.toml changed after the plan was made"),
        "{message}"
    );
    std::fs::write(&manifest, &declared).unwrap();
    // The target's catalog, changed outside migrations.
    f.execute(&format!("CREATE TABLE {s}.outside(id INTEGER)"));
    let message = stale(&migrate("test", "review.json"));
    assert!(
        message.starts_with("The target's catalog changed after the plan was made"),
        "{message}"
    );
    f.execute(&format!("DROP TABLE {s}.outside"));
    assert_eq!(f.command("status", &[])["history"]["applied_count"], 0);

    // Following the advice: the target the plan was reviewed for.
    let applied = migrate("test", "review.json");
    assert_eq!(applied["outcome"], "pass", "{applied}");
    // Now the target's history differs from the plan's.
    let r = f.command("verify", &["--against", "review.json"]);
    assert_eq!(r["plan_validation"]["outcome"], "fail", "{r}");
    assert!(stale(&r).starts_with("The target's history changed after the plan was made: the plan was made with 0 migration(s)"), "{r}");
    // An unreadable --against file is named as such, not as a stale plan.
    let r = f.command("verify", &["--against", "missing.json"]);
    assert_eq!(r["plan_validation"]["outcome"], "fail", "{r}");
    assert!(!code(&r, "DB_PLAN_STALE"), "{r}");
    let d = diagnostic(&r, "DB_INPUT_IO");
    assert_eq!(d["source"], "missing.json", "{d}");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("--against file missing.json: not found"),
        "{d}"
    );
    std::fs::write(f.root().join("truncated.json"), "{\"plan\":").unwrap();
    let r = f.command("plan", &["--against", "truncated.json"]);
    assert!(!code(&r, "DB_PLAN_STALE"), "{r}");
    assert_eq!(
        diagnostic(&r, "DB_PLAN_INVALID")["source"],
        "truncated.json",
        "{r}"
    );
}
fn contract_failures(create: fn() -> Fixture) {
    for (phase, check) in [
        ("requires", "SELECT false"),
        ("ensures", "SELECT false"),
        ("ensures", "SELECT NULL"),
        ("ensures", "SELECT 'true'"),
        ("ensures", "SELECT true, true"),
        ("ensures", "SELECT missing_column = 0"),
    ] {
        let f = create();
        f.sql(
            "0001_initial.sql",
            &format!("CREATE TABLE {}.items(id INTEGER)", f.schema()),
        );
        f.sql("0001_initial.toml", &format!("[{phase}]\ncheck={check:?}"));
        let plan = f.plan("plan.json");
        let failed = f.apply("plan.json", plan["plan_digest"].as_str());
        assert_eq!(failed["outcome"], "fail", "{failed}");
        assert_eq!(failed["assertions"]["outcome"], "fail");
        assert_eq!(failed["assertions"]["executed"], 1);
        assert_eq!(failed["progress"][0]["state"], "failed");
        let status = f.command("status", &[]);
        assert_eq!(status["history"]["applied_count"], 0, "{status}");
        assert_eq!(status["drift"]["outcome"], "pass");
    }
}
fn wrong_target(create: fn() -> Fixture) {
    let a = create();
    let b = create();
    let plan = first(&a);
    first(&b);
    std::fs::copy(a.root().join("initial.json"), b.root().join("wrong.json")).unwrap();
    let wrong = b.apply("wrong.json", plan["plan_digest"].as_str());
    assert_ne!(wrong["outcome"], "pass", "{wrong}");
    assert!(code(&wrong, "DB_PLAN_STALE") || code(&wrong, "DB_WRONG_TARGET"));
    assert_eq!(b.command("status", &[])["history"]["applied_count"], 0);
}

/// #428: a migration with no statements, such as the header `ntnt db new`
/// writes or a file of comments and semicolons, is refused as empty and named,
/// instead of as SQL outside the supported subset.
fn empty_migration(f: Fixture) {
    for sql in [
        "-- 0001_initial.sql: database \"app\" (sqlite).\n-- Write the SQL below. Once this migration has been applied anywhere,\n-- never edit or renumber it; add a new migration instead.\n",
        "/* nothing yet; */ ;\n\n-- SELECT 1;\n;",
    ] {
        f.sql("0001_initial.sql", sql);
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{r}");
        assert!(!code(&r, "DB_REPLAY_UNSUPPORTED"), "{r}");
        let d = r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["code"] == "DB_MIGRATION_EMPTY")
            .unwrap_or_else(|| panic!("no DB_MIGRATION_EMPTY in {r}"));
        assert_eq!(d["source"], "0001_initial.sql", "{d}");
    }
}

#[test]
fn sqlite_empty_migration() {
    empty_migration(Fixture::sqlite());
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_empty_migration() {
    empty_migration(Fixture::pg());
}

#[test]
fn sqlite_prefix_recovery() {
    prefix_recovery(Fixture::sqlite())
}
#[test]
fn sqlite_history_and_drift() {
    history_and_drift(Fixture::sqlite())
}
#[test]
fn sqlite_history_refusals_name_the_migration() {
    history_refusals_name_the_migration(Fixture::sqlite())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_history_refusals_name_the_migration() {
    history_refusals_name_the_migration(Fixture::pg())
}
#[test]
fn sqlite_stale_plans_name_what_changed() {
    stale_plans_name_what_changed(Fixture::sqlite())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_stale_plans_name_what_changed() {
    stale_plans_name_what_changed(Fixture::pg())
}
/// #423: a PostgreSQL target whose session settings changed after the plan
/// names the setting with both values.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_a_changed_target_setting_is_named() {
    let pg = pg_fixture::Fixture::new();
    pg.sql(
        "0001_initial.sql",
        "CREATE TABLE app.items(id INTEGER PRIMARY KEY);",
    );
    let plan = pg.plan("review.json");
    let before = plan["target"]["identity"]["settings"]["timezone"]
        .as_str()
        .unwrap()
        .to_owned();
    let zone = if before == "Asia/Tokyo" {
        "Europe/Paris"
    } else {
        "Asia/Tokyo"
    };
    // Both roles, as scratch must match the target's settings to replay.
    for (url, role) in [(&pg.target_url, &pg.target), (&pg.scratch_url, &pg.scratch)] {
        postgres::Client::connect(url, postgres::NoTls)
            .unwrap()
            .batch_execute(&format!("ALTER ROLE {role} SET timezone = '{zone}'"))
            .unwrap();
    }
    let expected = format!("target setting timezone: plan {before}, now {zone}");
    let r = pg.apply("review.json", plan["plan_digest"].as_str());
    let d = diagnostic(&r, "DB_PLAN_STALE");
    assert!(d["message"].as_str().unwrap().contains(&expected), "{d}");
    let r = pg.command_args("plan", &["--against", "review.json"]);
    assert_eq!(r["plan_validation"]["outcome"], "fail", "{r}");
    let d = diagnostic(&r, "DB_PLAN_STALE");
    assert!(d["message"].as_str().unwrap().contains(&expected), "{d}");
}
#[test]
fn sqlite_contract_failures() {
    contract_failures(Fixture::sqlite)
}
#[test]
fn sqlite_wrong_target() {
    wrong_target(Fixture::sqlite)
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_prefix_recovery() {
    prefix_recovery(Fixture::pg())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_history_and_drift() {
    history_and_drift(Fixture::pg())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_contract_failures() {
    contract_failures(Fixture::pg)
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_wrong_target() {
    wrong_target(Fixture::pg)
}

fn adoption_checks(f: &Fixture, sql: &str) {
    use std::io::Write;
    let mut manifest = std::fs::OpenOptions::new()
        .append(true)
        .open(f.root().join("ntnt.toml"))
        .unwrap();
    writeln!(
        manifest,
        "\n[databases.app.adoption_checks]\nrepresentative={sql:?}"
    )
    .unwrap();
}
fn adopt_populated_checkpoint_then_upgrade(f: Fixture) {
    let initial = format!(
        "CREATE TABLE {}.items(id INTEGER PRIMARY KEY,value TEXT);",
        f.schema()
    );
    f.sql("0001_baseline.sql", &initial);
    f.execute(&initial);
    f.execute(&format!(
        "INSERT INTO {}.items VALUES(17,'retained')",
        f.schema()
    ));
    assert!(code(&f.command("status", &[]), "DB_UNRECOGNIZED_HISTORY"));
    adoption_checks(
        &f,
        &format!(
            "SELECT NOT EXISTS (SELECT 1 FROM {}.items WHERE value IS NULL)",
            f.schema()
        ),
    );
    let plan = f.command("plan", &["--adopt-through", "1", "--out", "adoption.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(plan["plan"]["action"]["kind"], "adopt_baseline");
    assert_eq!(
        plan["capabilities"]["operations"]["baseline_adoption"],
        "reviewed_prefix_checkpoint"
    );
    assert_eq!(plan["assertions"]["executed"], 0);
    assert!(code(
        &f.apply("adoption.json", None),
        "DB_APPROVAL_REQUIRED"
    ));
    let adopted = f.apply("adoption.json", plan["plan_digest"].as_str());
    assert_eq!(adopted["outcome"], "pass", "{adopted}");
    assert_eq!(adopted["assertions"]["executed"], 1);
    assert_eq!(adopted["progress"][0]["kind"], "adopt_baseline");
    assert_eq!(adopted["progress"][0]["state"], "committed");
    let status = f.command("verify", &[]);
    assert_eq!(status["outcome"], "pass", "{status}");
    assert_eq!(status["history"]["applied_count"], 0);
    assert_eq!(status["history"]["adopted_count"], 1);
    assert_eq!(status["history"]["covered_count"], 1);
    assert_eq!(
        status["history"]["ledger"]["entries"][0]["state"],
        "adopted"
    );
    assert_eq!(f.count(&format!("SELECT count(*) FROM {}", f.ledger())), 1);
    assert_eq!(
        f.text(&format!(
            "SELECT value FROM {}.items WHERE id=17",
            f.schema()
        )),
        "retained"
    );
    // Adoption checks are an observed transition, not persistent invariants.
    f.execute(&format!("INSERT INTO {}.items VALUES(18,NULL)", f.schema()));
    let again = f.apply("adoption.json", plan["plan_digest"].as_str());
    assert_eq!(again["outcome"], "pass", "{again}");
    assert_eq!(again["assertions"]["executed"], 0);
    assert_eq!(again["progress"][0]["state"], "already_adopted");
    f.sql(
        "0002_upgrade.sql",
        &format!(
            "ALTER TABLE {}.items ADD COLUMN label TEXT DEFAULT 'expanded';",
            f.schema()
        ),
    );
    let upgrade = f.plan("upgrade.json");
    let applied = f.apply("upgrade.json", upgrade["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(applied["progress"][0]["state"], "baseline_covered");
    assert_eq!(applied["progress"][1]["state"], "committed");
    assert_eq!(
        f.text(&format!(
            "SELECT label FROM {}.items WHERE id=17",
            f.schema()
        )),
        "expanded"
    );
    let verified = f.command("verify", &[]);
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["history"]["covered_count"], 2);
    assert_eq!(verified["history"]["adopted_count"], 1);
    assert_eq!(verified["history"]["applied_count"], 1);
}
#[test]
fn sqlite_adopts_populated_checkpoint_then_upgrades() {
    adopt_populated_checkpoint_then_upgrade(Fixture::sqlite())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_adopts_populated_checkpoint_then_upgrades() {
    adopt_populated_checkpoint_then_upgrade(Fixture::pg())
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_imports_only_verified_legacy_history_without_replaying_sql() {
    let pg = pg_fixture::Fixture::new();
    let initial = pg.seed_first("CREATE TABLE app.items(id INTEGER PRIMARY KEY,value TEXT);");
    let checksum = initial["source"]["migrations"][0]["checksum"]
        .as_str()
        .unwrap()
        .to_owned();
    pg.target()
        .batch_execute("INSERT INTO app.items VALUES(17,'preserved')")
        .unwrap();
    let f = Fixture::Postgres(Box::new(pg));
    adoption_checks(
        &f,
        "SELECT NOT EXISTS (SELECT 1 FROM app.items WHERE value IS NULL)",
    );
    let ordinary = f.plan("ordinary.json");
    assert!(code(
        &f.apply("ordinary.json", ordinary["plan_digest"].as_str()),
        "DB_LEDGER_SCOPE"
    ));
    let plan = f.command("plan", &["--adopt-through", "1", "--out", "import.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(plan["plan"]["action"]["kind"], "import_legacy_v1");
    let checked = f.command("verify", &["--against", "import.json"]);
    assert_eq!(checked["outcome"], "pass", "{checked}");
    assert_eq!(checked["plan_validation"]["outcome"], "pass");
    assert_eq!(checked["assertions"]["executed"], 0);
    assert_eq!(
        f.text("SELECT state FROM _ntnt.schema_migrations_v1"),
        "applied"
    );
    assert!(code(&f.apply("import.json", None), "DB_APPROVAL_REQUIRED"));
    let imported = f.apply("import.json", plan["plan_digest"].as_str());
    assert_eq!(imported["outcome"], "pass", "{imported}");
    assert_eq!(imported["assertions"]["executed"], 1);
    assert_eq!(imported["progress"][0]["kind"], "import_legacy_v1");
    assert_eq!(
        f.text("SELECT checksum FROM _ntnt.schema_migrations_v2"),
        checksum
    );
    assert_eq!(
        f.text("SELECT state FROM _ntnt.schema_migrations_v2"),
        "imported"
    );
    assert_eq!(
        f.text("SELECT value FROM app.items WHERE id=17"),
        "preserved"
    );
    let verified = f.command("verify", &[]);
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["history"]["applied_count"], 0);
    assert_eq!(verified["history"]["imported_count"], 1);
    assert_eq!(verified["history"]["adopted_count"], 0);
    assert_eq!(verified["history"]["covered_count"], 1);
    assert_eq!(
        f.apply("import.json", plan["plan_digest"].as_str())["progress"][0]["state"],
        "already_imported"
    );
    f.sql(
        "0002_upgrade.sql",
        "ALTER TABLE app.items ADD COLUMN label TEXT DEFAULT 'after-import';",
    );
    let plan = f.plan("upgrade.json");
    let upgraded = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(upgraded["outcome"], "pass", "{upgraded}");
    assert_eq!(upgraded["progress"][0]["state"], "history_imported");
    assert_eq!(upgraded["progress"][1]["state"], "committed");
    assert_eq!(
        f.text("SELECT label FROM app.items WHERE id=17"),
        "after-import"
    );
    assert_eq!(f.command("verify", &[])["history"]["applied_count"], 1);
}

fn untracked(f: &Fixture) {
    let sql = format!(
        "CREATE TABLE {}.items(id INTEGER PRIMARY KEY,value TEXT);",
        f.schema()
    );
    f.sql("0001_initial.sql", &sql);
    f.execute(&sql);
    f.execute(&format!(
        "INSERT INTO {}.items VALUES(17,'retained')",
        f.schema()
    ));
}
fn assert_unadopted(f: &Fixture) {
    let status = f.command("status", &[]);
    assert!(code(&status, "DB_UNRECOGNIZED_HISTORY"), "{status}");
    let guidance = status["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_UNRECOGNIZED_HISTORY")
        .unwrap()["next_action"]
        .as_str()
        .unwrap();
    assert!(
        guidance.contains("plan --adopt-through") && guidance.contains("adoption_checks"),
        "{guidance}"
    );
    assert!(!guidance.contains("not implemented"), "{guidance}");
    assert_eq!(status["history"]["ledger"]["present"], false);
    assert_eq!(status["history"]["covered_count"], 0);
}
fn adoption_covers_an_exact_multi_file_prefix(f: Fixture) {
    untracked(&f);
    f.sql(
        "0001_initial.toml",
        "[requires]\nhistorical='SELECT false'\n[ensures]\nsuperseded='SELECT false'",
    );
    let second = format!(
        "ALTER TABLE {}.items ADD COLUMN label TEXT DEFAULT 'legacy';",
        f.schema()
    );
    f.sql("0010_second.sql", &second);
    f.execute(&second);
    // Pending unsupported work cannot be replayed by an adoption-only plan.
    f.sql("0020_pending.sql", "VACUUM;");
    adoption_checks(&f, &format!("SELECT count(*)=1 FROM {}.items", f.schema()));
    let plan = f.command("plan", &["--adopt-through", "10", "--out", "adopt.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(plan["plan"]["action"]["count"], 2);
    assert_eq!(
        plan["plan"]["execution"]["prefix_fingerprints"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(plan["plan"]["pending"], serde_json::json!([]));
    // Changed rows deliberately make the adoption check false. Readonly
    // revalidation must still check only schema/history/input identity.
    f.execute(&format!(
        "INSERT INTO {}.items(id,value) VALUES(18,'not-part-of-the-plan')",
        f.schema()
    ));
    for command in ["verify", "status", "plan"] {
        let checked = f.command(command, &["--against", "adopt.json"]);
        assert_eq!(checked["outcome"], "pass", "{checked}");
        assert_eq!(checked["plan_validation"]["outcome"], "pass");
        assert_eq!(checked["plan_validation"]["authorized"], false);
        assert_eq!(checked["assertions"]["executed"], 0);
        if command == "plan" {
            assert_eq!(checked["plan_digest"], plan["plan_digest"]);
        }
    }
    f.execute(&format!("DELETE FROM {}.items WHERE id=18", f.schema()));
    assert_unadopted(&f);
    let adopted = f.apply("adopt.json", plan["plan_digest"].as_str());
    assert_eq!(adopted["outcome"], "pass", "{adopted}");
    assert_eq!(adopted["progress"][0]["migration_sql_executed"], false);
    assert_eq!(adopted["assertions"]["executed"], 1); // no historical transition checks
    let status = f.command("verify", &[]);
    assert_eq!(status["outcome"], "pass", "{status}");
    assert_eq!(status["history"]["adopted_count"], 2);
    assert_eq!(status["history"]["applied_count"], 0);
    assert_eq!(status["history"]["pending_count"], 1);
    assert_eq!(
        status["history"]["ledger"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(f.count(&format!("SELECT position FROM {}", f.ledger())), 2);
    assert_eq!(f.text(&format!("SELECT id FROM {}", f.ledger())), "10");
    assert!(code(&f.command("plan", &[]), "DB_REPLAY_UNSUPPORTED"));
    assert!(code(
        &f.command("plan", &["--adopt-through", "10"]),
        "DB_ADOPTION_INPUT"
    ));
    let original = std::fs::read_to_string(f.root().join("migrations/0001_initial.sql")).unwrap();
    f.sql("0001_initial.sql", &(original.clone() + " -- changed"));
    assert!(code(&f.command("verify", &[]), "DB_HISTORY_EDITED"));
    f.sql("0001_initial.sql", &original);
    f.sql(
        "0005_inserted.sql",
        &format!("CREATE TABLE {}.inserted(id INTEGER)", f.schema()),
    );
    assert!(code(&f.command("verify", &[]), "DB_HISTORY_DIVERGENT"));
    std::fs::remove_file(f.root().join("migrations/0005_inserted.sql")).unwrap();
    std::fs::remove_file(f.root().join("migrations/0010_second.sql")).unwrap();
    assert!(code(&f.command("verify", &[]), "DB_HISTORY_DIVERGENT"));
    assert_eq!(
        f.text(&format!(
            "SELECT value FROM {}.items WHERE id=17",
            f.schema()
        )),
        "retained"
    );
}
fn adoption_checks_fail_without_bootstrap(create: fn() -> Fixture) {
    for sql in [
        "SELECT false",
        "SELECT NULL",
        "SELECT 'true'",
        "SELECT true,true",
        "SELECT missing_column=0",
        "SELECT true FROM SCHEMA.items WHERE id<0",
        "SELECT true FROM SCHEMA.items",
    ] {
        let f = create();
        untracked(&f);
        f.execute(&format!(
            "INSERT INTO {}.items VALUES(18,'also-retained')",
            f.schema()
        ));
        adoption_checks(&f, &sql.replace("SCHEMA", f.schema()));
        let plan = f.command("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
        assert_eq!(plan["outcome"], "pass", "{plan}");
        assert_eq!(plan["assertions"]["executed"], 0);
        let failed = f.apply("adopt.json", plan["plan_digest"].as_str());
        assert_eq!(failed["outcome"], "fail", "{failed}");
        assert_eq!(failed["assertions"]["executed"], 1);
        assert_eq!(failed["assertions"]["outcome"], "fail");
        assert_eq!(failed["progress"][0]["state"], "failed");
        assert_unadopted(&f);
        assert_eq!(
            f.count(&format!("SELECT count(*) FROM {}.items", f.schema())),
            2
        );
    }
}
fn adoption_rechecks_populated_data(f: Fixture) {
    untracked(&f);
    adoption_checks(&f,&format!("SELECT NOT EXISTS (SELECT 1 FROM {s}.items WHERE value IS NULL) AND NOT EXISTS (SELECT value FROM {s}.items GROUP BY value HAVING count(*)>1)",s=f.schema()));
    let plan = f.command("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    f.execute(&format!(
        "INSERT INTO {}.items VALUES(18,'retained'),(19,NULL)",
        f.schema()
    ));
    assert!(code(
        &f.apply("adopt.json", plan["plan_digest"].as_str()),
        "DB_CHECK_FAILED"
    ));
    assert_unadopted(&f);
    f.execute(&format!(
        "UPDATE {}.items SET value='nonnull' WHERE id=19",
        f.schema()
    ));
    assert!(code(
        &f.apply("adopt.json", plan["plan_digest"].as_str()),
        "DB_CHECK_FAILED"
    ));
    assert_unadopted(&f);
    f.execute(&format!(
        "UPDATE {}.items SET value='distinct' WHERE id=18",
        f.schema()
    ));
    assert_eq!(
        f.apply("adopt.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
    assert_eq!(
        f.text(&format!(
            "SELECT value FROM {}.items WHERE id=17",
            f.schema()
        )),
        "retained"
    );
    assert_eq!(
        f.count(&format!("SELECT count(*) FROM {}.items", f.schema())),
        3
    );
}
fn adoption_binding_and_coverage(create: fn() -> Fixture) {
    let empty = create();
    empty.sql(
        "0001_initial.sql",
        &format!(
            "CREATE TABLE {}.items(id INTEGER PRIMARY KEY,value TEXT);",
            empty.schema()
        ),
    );
    adoption_checks(&empty, "SELECT true");
    let mismatch = empty.command("plan", &["--adopt-through", "1"]);
    assert!(code(&mismatch, "DB_DRIFT"), "{mismatch}");
    // A fresh target uses normal migration execution, not implicit adoption.
    let fresh = empty.plan("fresh.json");
    assert_eq!(
        empty.apply("fresh.json", fresh["plan_digest"].as_str())["outcome"],
        "pass"
    );
    assert_eq!(empty.command("verify", &[])["history"]["adopted_count"], 0);
    let f = create();
    untracked(&f);
    assert!(code(
        &f.command("plan", &["--adopt-through", "1"]),
        "DB_ADOPTION_INPUT"
    ));
    adoption_checks(&f, "SELECT true");
    assert!(code(
        &f.command("plan", &["--adopt-through", "2"]),
        "DB_ADOPTION_INPUT"
    ));
    let plan = f.command("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let other = create();
    untracked(&other);
    adoption_checks(&other, "SELECT true");
    std::fs::copy(f.root().join("adopt.json"), other.root().join("wrong.json")).unwrap();
    let wrong = other.apply("wrong.json", plan["plan_digest"].as_str());
    assert!(
        code(&wrong, "DB_PLAN_STALE") || code(&wrong, "DB_WRONG_TARGET"),
        "{wrong}"
    );
    assert_unadopted(&other);
    let original = std::fs::read_to_string(f.root().join("migrations/0001_initial.sql")).unwrap();
    f.sql("0001_initial.sql", &(original.clone() + " -- changed"));
    assert!(code(
        &f.apply("adopt.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    f.sql("0001_initial.sql", &original);
    let manifest_path = f.root().join("ntnt.toml");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();
    let mut policy: toml::Value = manifest.parse().unwrap();
    policy["databases"]["app"]["adoption_checks"]["representative"] =
        toml::Value::String("SELECT false".into());
    std::fs::write(&manifest_path, toml::to_string(&policy).unwrap()).unwrap();
    assert!(code(
        &f.apply("adopt.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    std::fs::write(&manifest_path, manifest).unwrap();
    f.execute(&format!(
        "ALTER TABLE {}.items ADD COLUMN outside TEXT",
        f.schema()
    ));
    assert!(code(
        &f.apply("adopt.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    assert_unadopted(&f);
    let mismatched = f.command("plan", &["--adopt-through", "1"]);
    assert!(code(&mismatched, "DB_DRIFT"), "{mismatched}");
    assert_eq!(mismatched["drift"]["outcome"], "fail");
}
fn adoption_never_repairs_unknown_ledgers(f: Fixture) {
    untracked(&f);
    adoption_checks(&f, "SELECT true");
    let (setup,query)=match &f {
        Fixture::Postgres(_)=>("CREATE SCHEMA _ntnt; CREATE TABLE _ntnt.unknown(value BIGINT); INSERT INTO _ntnt.unknown VALUES(17)","SELECT value FROM _ntnt.unknown"),
        Fixture::Sqlite(_)=>("CREATE TABLE _ntnt_unknown(value INTEGER); INSERT INTO _ntnt_unknown VALUES(17)","SELECT value FROM _ntnt_unknown"),
    };
    f.execute(setup);
    let rejected = f.command("plan", &["--adopt-through", "1"]);
    assert!(code(&rejected, "DB_LEDGER_UNKNOWN"), "{rejected}");
    let guidance = rejected["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_LEDGER_UNKNOWN")
        .unwrap()["next_action"]
        .as_str()
        .unwrap();
    assert!(
        guidance.contains("operator") && guidance.contains("never reset"),
        "{guidance}"
    );
    assert!(
        !guidance.contains("plan --adopt-through") && !guidance.contains("not implemented"),
        "{guidance}"
    );
    assert_eq!(f.count(query), 17);
    assert_eq!(
        f.text(&format!(
            "SELECT value FROM {}.items WHERE id=17",
            f.schema()
        )),
        "retained"
    );
}
#[test]
fn sqlite_adoption_multi_file_prefix() {
    adoption_covers_an_exact_multi_file_prefix(Fixture::sqlite())
}
#[test]
fn sqlite_adoption_check_failures() {
    adoption_checks_fail_without_bootstrap(Fixture::sqlite)
}
#[test]
fn sqlite_adoption_rechecks_populated_data() {
    adoption_rechecks_populated_data(Fixture::sqlite())
}
#[test]
fn sqlite_adoption_binding_and_coverage() {
    adoption_binding_and_coverage(Fixture::sqlite)
}
#[test]
fn sqlite_adoption_unknown_ledger() {
    adoption_never_repairs_unknown_ledgers(Fixture::sqlite())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_adoption_multi_file_prefix() {
    adoption_covers_an_exact_multi_file_prefix(Fixture::pg())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_adoption_check_failures() {
    adoption_checks_fail_without_bootstrap(Fixture::pg)
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_adoption_rechecks_populated_data() {
    adoption_rechecks_populated_data(Fixture::pg())
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_adoption_binding_and_coverage() {
    adoption_binding_and_coverage(Fixture::pg)
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_adoption_unknown_ledger() {
    adoption_never_repairs_unknown_ledgers(Fixture::pg())
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_legacy_adoption_rejects_edited_dirty_and_ambiguous_history() {
    for (sql, expected) in [
        (
            "UPDATE _ntnt.schema_migrations_v1 SET checksum=repeat('a',64)",
            "DB_HISTORY_EDITED",
        ),
        (
            "UPDATE _ntnt.schema_migrations_v1 SET state='running'",
            "DB_HISTORY_DIRTY",
        ),
        (
            "UPDATE _ntnt.schema_migrations_v1 SET id='2'",
            "DB_HISTORY_DIVERGENT",
        ),
        (
            "DELETE FROM _ntnt.schema_migrations_v1",
            "DB_ADOPTION_INPUT",
        ),
    ] {
        let pg = pg_fixture::Fixture::new();
        pg.seed_first("CREATE TABLE app.items(id INTEGER PRIMARY KEY,value TEXT);");
        pg.target()
            .batch_execute("INSERT INTO app.items VALUES(17,'retained')")
            .unwrap();
        let f = Fixture::Postgres(Box::new(pg));
        adoption_checks(&f, "SELECT true");
        f.execute(sql);
        let rejected = f.command("plan", &["--adopt-through", "1"]);
        assert!(code(&rejected, expected), "{rejected}");
        if expected == "DB_ADOPTION_INPUT" {
            // #422: the refusal says what makes the ledger ineligible.
            let d = rejected["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["code"] == expected)
                .unwrap();
            assert!(
                d["message"]
                    .as_str()
                    .unwrap()
                    .contains("legacy v1 ledger is empty"),
                "{d}"
            );
        }
        assert_eq!(
            f.text("SELECT value FROM app.items WHERE id=17"),
            "retained"
        );
        assert_eq!(f.count("SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='_ntnt' AND c.relname='schema_migrations_v2'"),0);
    }
}
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI conformance"]
fn postgres_legacy_adoption_requires_a_mutation_safe_owned_ledger() {
    for sql in [
        "ALTER TABLE _ntnt.schema_migrations_v1 ALTER COLUMN id SET DEFAULT 'unused'",
        "ALTER TABLE _ntnt.schema_migrations_v1 ADD CHECK(position>0)",
        "ALTER TABLE _ntnt.schema_migrations_v1 SET UNLOGGED",
        "GRANT SELECT ON _ntnt.schema_migrations_v1 TO PUBLIC",
    ] {
        let pg = pg_fixture::Fixture::new();
        let initial = pg.seed_first("CREATE TABLE app.items(id INTEGER PRIMARY KEY,value TEXT);");
        pg.target()
            .batch_execute("INSERT INTO app.items VALUES(17,'retained')")
            .unwrap();
        let f = Fixture::Postgres(Box::new(pg));
        adoption_checks(&f, "SELECT true");
        let plan = f.command("plan", &["--adopt-through", "1", "--out", "import.json"]);
        assert_eq!(plan["outcome"], "pass", "{plan}");
        f.execute(sql);
        let failed = f.apply("import.json", plan["plan_digest"].as_str());
        assert!(code(&failed, "DB_LEDGER_UNSAFE"), "{failed}");
        assert_eq!(failed["progress"][0]["state"], "failed");
        assert_eq!(
            f.text("SELECT checksum FROM _ntnt.schema_migrations_v1"),
            initial["source"]["migrations"][0]["checksum"]
                .as_str()
                .unwrap()
        );
        assert_eq!(
            f.text("SELECT state FROM _ntnt.schema_migrations_v1"),
            "applied"
        );
        assert_eq!(
            f.text("SELECT value FROM app.items WHERE id=17"),
            "retained"
        );
        assert_eq!(f.count("SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='_ntnt' AND c.relname='schema_migrations_v2'"),0);
    }
}
