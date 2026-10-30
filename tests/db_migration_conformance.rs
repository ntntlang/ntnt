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
