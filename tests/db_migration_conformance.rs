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

#[test]
fn sqlite_prefix_recovery() {
    prefix_recovery(Fixture::sqlite())
}
#[test]
fn sqlite_history_and_drift() {
    history_and_drift(Fixture::sqlite())
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
