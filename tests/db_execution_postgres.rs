//! Real fresh/populated rehearsal uses the production CLI and executor.
#[path = "support/db_fixture.rs"]
mod db_fixture;
use db_fixture::Fixture;
use serde_json::Value;

fn code(r: &Value, code: &str) -> bool {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn exact_authority_fresh_execution_and_unchanged_rerun() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts(id INTEGER PRIMARY KEY, label TEXT DEFAULT 'a; b');",
    );
    let p = f.plan("plan.json");
    for approval in [None, Some("not-the-reviewed-digest")] {
        let r = f.apply("plan.json", approval);
        assert!(code(&r, "DB_APPROVAL_REQUIRED"), "{r}");
        assert!(!f
            .target()
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='_ntnt')",
                &[]
            )
            .unwrap()
            .get::<_, bool>(0));
    }
    let digest = p["plan_digest"].as_str().unwrap();
    let r = f.apply("plan.json", Some(digest));
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
    assert_eq!(r["target"]["identity"], p["target"]["identity"], "{r}");
    assert_eq!(r["source"]["input_digest"], p["source"]["input_digest"]);
    f.target()
        .batch_execute("INSERT INTO app.accounts(id) VALUES(17)")
        .unwrap();
    let again = f.apply("plan.json", Some(digest));
    assert_eq!(again["outcome"], "pass", "{again}");
    assert_eq!(again["progress"][0]["state"], "already_applied");
    assert_eq!(
        f.target()
            .query_one("SELECT label FROM app.accounts WHERE id=17", &[])
            .unwrap()
            .get::<_, String>(0),
        "a; b"
    );
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn populated_upgrade_checks_are_atomic_and_resume_a_committed_prefix() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts(id INTEGER PRIMARY KEY, email TEXT);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .batch_execute(
            "INSERT INTO app.accounts VALUES(17,'alice@example.test'),(-1,'negative@example.test')",
        )
        .unwrap();
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.accounts ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';",
    );
    std::fs::write(f.root.path().join("migrations/0002_expand.toml"), r#"
[requires]
no_nulls = "SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE email IS NULL)"
no_duplicates = "SELECT NOT EXISTS (SELECT email FROM app.accounts GROUP BY email HAVING count(*) > 1)"
[ensures]
filled = "SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE timezone <> 'UTC')"
"#).unwrap();
    f.sql(
        "0003_contract.sql",
        "ALTER TABLE app.accounts ADD COLUMN active BOOLEAN DEFAULT true;",
    );
    std::fs::write(
        f.root.path().join("migrations/0003_contract.toml"),
        r#"
[ensures]
positive = "SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE id < 0)"
"#,
    )
    .unwrap();
    let plan = f.plan("upgrade.json");
    assert_eq!(plan["assertions"]["outcome"], "not_run");
    let failed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(failed["progress"][1]["state"], "committed", "{failed}");
    assert_eq!(failed["progress"][2]["state"], "failed", "{failed}");
    assert_eq!(failed["assertions"]["executed"], 4);
    assert!(code(&failed, "DB_CHECK_FAILED"), "{failed}");
    assert_eq!(
        f.target()
            .query_one("SELECT email,timezone FROM app.accounts WHERE id=17", &[])
            .unwrap()
            .get::<_, String>(0),
        "alice@example.test"
    );
    assert!(!f.target().query_one("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema='app' AND table_name='accounts' AND column_name='active')",&[]).unwrap().get::<_,bool>(0));
    f.target()
        .batch_execute("DELETE FROM app.accounts WHERE id=-1")
        .unwrap();
    let resumed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(resumed["outcome"], "pass", "{resumed}");
    assert_eq!(resumed["progress"][1]["state"], "already_applied");
    assert_eq!(resumed["progress"][2]["state"], "committed");
    assert_eq!(resumed["assertions"]["executed"], 1);
    f.target()
        .batch_execute("UPDATE app.accounts SET timezone='America/Denver'")
        .unwrap();
    let verified = f.command("verify");
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["assertions"]["executed"], 0);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_fail_closed_for_false_null_shape_type_error_and_preconditions() {
    for (phase, sql) in [
        ("requires", "SELECT false"),
        ("ensures", "SELECT false"),
        ("ensures", "SELECT NULL"),
        ("ensures", "SELECT true, true"),
        ("ensures", "SELECT 1"),
        ("ensures", "SELECT 1 / 0 = 0"),
        ("ensures", "SELECT true FROM app.t"),
    ] {
        let f = Fixture::new();
        f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
        std::fs::write(
            f.root.path().join("migrations/0001_initial.toml"),
            format!("[{phase}]\ncheck = {sql:?}\n"),
        )
        .unwrap();
        let plan = f.plan("plan.json");
        let r = f.apply("plan.json", plan["plan_digest"].as_str());
        assert_eq!(r["outcome"], "fail", "{phase} {sql}: {r}");
        assert_eq!(r["assertions"]["executed"], 1, "{r}");
        assert_eq!(r["assertions"]["outcome"], "fail");
        assert_eq!(r["progress"][0]["state"], "failed");
        assert!(!f
            .target()
            .query_one("SELECT to_regclass('app.t') IS NOT NULL", &[])
            .unwrap()
            .get::<_, bool>(0));
        assert!(!f
            .target()
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='_ntnt')",
                &[]
            )
            .unwrap()
            .get::<_, bool>(0));
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_reject_unsupported_effect_paths_without_executing_them() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    for sql in [
        "SELECT pg_advisory_lock(245)",
        "SELECT pg_sleep(30) IS NULL",
        "SELECT true INTO app.canary",
        "SELECT true FROM app.t FOR UPDATE",
        "WITH deleted AS (DELETE FROM app.t RETURNING id) SELECT true",
        "SELECT 't'::boolean",
        "SELECT true FROM pg_catalog.pg_class",
        "SELECT app.side_effect()",
        "SELECT true; COMMIT",
        "SELECT true FROM app.side_effect()",
        "SELECT true FROM app.t TABLESAMPLE SYSTEM(10)",
    ] {
        std::fs::write(
            f.root.path().join("migrations/0001_initial.toml"),
            format!("[requires]\ncheck={sql:?}\n"),
        )
        .unwrap();
        let r = f.command("plan");
        assert!(code(&r, "DB_CHECK_UNSUPPORTED"), "{sql}: {r}");
        assert_eq!(r["assertions"]["executed"], 0);
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn contract_cannot_read_a_system_table_instead_of_managed_rows() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.pg_class(oid INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .batch_execute("INSERT INTO app.pg_class VALUES(0)")
        .unwrap();
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.pg_class ADD COLUMN checked BOOLEAN DEFAULT true;",
    );
    let metadata = f.root.path().join("migrations/0002_expand.toml");
    std::fs::write(
        &metadata,
        "[requires]\nempty='SELECT NOT EXISTS (SELECT 1 FROM pg_class WHERE oid = 0)'",
    )
    .unwrap();
    let mut rejected = f.command("plan");
    // Before the fix this produced an executable plan and incorrectly committed
    // against pg_catalog.pg_class. Exercise that real path, not a parser mock.
    if rejected["outcome"] == "pass" {
        std::fs::write(
            f.root.path().join("unqualified.json"),
            serde_json::to_vec(&rejected).unwrap(),
        )
        .unwrap();
        rejected = f.apply("unqualified.json", rejected["plan_digest"].as_str());
    }
    assert!(code(&rejected, "DB_CHECK_UNSUPPORTED"), "{rejected}");
    assert_eq!(rejected["assertions"]["executed"], 0);

    std::fs::write(
        &metadata,
        "[requires]\nempty='SELECT NOT EXISTS (SELECT 1 FROM app.pg_class WHERE oid = 0)'",
    )
    .unwrap();
    let plan = f.plan("qualified.json");
    let failed = f.apply("qualified.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_CHECK_FAILED"), "{failed}");
    assert_eq!(failed["progress"][1]["state"], "failed");
    assert_eq!(failed["assertions"]["executed"], 1);
    assert!(!f.target().query_one("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema='app' AND table_name='pg_class' AND column_name='checked')",&[]).unwrap().get::<_,bool>(0));
    f.target()
        .batch_execute("DELETE FROM app.pg_class")
        .unwrap();
    assert_eq!(
        f.apply("qualified.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn stale_and_incomplete_execution_artifacts_never_bootstrap() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let plan = f.plan("plan.json");
    let mut old = plan.clone();
    old["plan"].as_object_mut().unwrap().remove("execution");
    std::fs::write(
        f.root.path().join("old.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    assert!(code(
        &f.apply("old.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id BIGINT);");
    assert!(code(
        &f.apply("plan.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    f.target()
        .batch_execute("CREATE TABLE app.unrelated(id INTEGER)")
        .unwrap();
    assert!(code(
        &f.apply("plan.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    assert!(!f
        .target()
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='_ntnt')",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn casts_outside_managed_scope_cannot_invoke_user_code_during_ddl() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .batch_execute("INSERT INTO app.t VALUES(17)")
        .unwrap();
    f.sql(
        "0002_cast.sql",
        "ALTER TABLE app.t ALTER COLUMN id TYPE TEXT;",
    );
    let plan = f.plan("cast.json");
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    config.dbname(&f.target);
    let mut admin = config.connect(postgres::NoTls).unwrap();
    admin.batch_execute("CREATE FUNCTION public.escape(integer) RETURNS text LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_lock(87654); RETURN 'bad'; END $$; CREATE CAST (integer AS text) WITH FUNCTION public.escape(integer) AS ASSIGNMENT").unwrap();
    let r = f.apply("cast.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_CHECK_UNSUPPORTED"), "{r}");
    assert_eq!(
        f.target()
            .query_one("SELECT id FROM app.t", &[])
            .unwrap()
            .get::<_, i32>(0),
        17
    );
}

#[cfg(unix)]
#[path = "support/db_proxy.rs"]
mod db_proxy;

#[cfg(unix)]
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn commit_disconnects_reconcile_fresh_without_reexecution_or_losing_prefix() {
    use db_proxy::{Fault, Proxy};
    for fault in [
        Fault::BeforeCommit,
        Fault::AfterCommit,
        Fault::AfterCommitUnavailable,
    ] {
        let mut f = Fixture::new();
        f.sql(
            "0001_initial.sql",
            "CREATE TABLE app.t(id INTEGER PRIMARY KEY);",
        );
        f.sql(
            "0002_expand.sql",
            "ALTER TABLE app.t ADD COLUMN label TEXT DEFAULT 'once';",
        );
        f.sql(
            "0003_finish.sql",
            "ALTER TABLE app.t ADD COLUMN active BOOLEAN DEFAULT true;",
        );
        let plan = f.plan("plan.json");
        let proxy = Proxy::new(fault, 2);
        let direct = f.target_url.clone();
        f.target_url = direct.replace(&std::env::var("NTNT_DB_TEST_HOST").unwrap(), proxy.host());
        let r = f.apply("plan.json", plan["plan_digest"].as_str());
        assert!(proxy.fired.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            r["reconciliation"]["outcome"],
            if matches!(fault, Fault::AfterCommitUnavailable) {
                "unknown"
            } else {
                "pass"
            },
            "{r}"
        );
        assert_eq!(r["progress"][0]["state"], "committed", "{r}");
        assert_eq!(r["progress"][2]["state"], "not_run", "{r}");
        assert_eq!(
            r["progress"][1]["state"],
            if matches!(fault, Fault::AfterCommitUnavailable) {
                "unknown"
            } else if matches!(fault, Fault::AfterCommit) {
                "committed"
            } else {
                "failed"
            },
            "{r}"
        );
        f.target_url = direct;
        f.target()
            .batch_execute("INSERT INTO app.t(id) VALUES(17)")
            .unwrap();
        let retry = f.apply("plan.json", plan["plan_digest"].as_str());
        assert_eq!(retry["outcome"], "pass", "{retry}");
        assert_eq!(
            f.target()
                .query_one("SELECT count(*) FROM _ntnt.schema_migrations_v2", &[])
                .unwrap()
                .get::<_, i64>(0),
            3
        );
        assert_eq!(
            f.target()
                .query_one("SELECT label FROM app.t WHERE id=17", &[])
                .unwrap()
                .get::<_, String>(0),
            "once"
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn rollback_disconnect_discards_connection_and_reports_unknown_cleanup() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    std::fs::write(
        f.root.path().join("migrations/0001_initial.toml"),
        "[ensures]\nfailed='SELECT false'",
    )
    .unwrap();
    let plan = f.plan("plan.json");
    let proxy = db_proxy::Proxy::new(db_proxy::Fault::Rollback, 0);
    let direct = f.target_url.clone();
    f.target_url = direct.replace(&std::env::var("NTNT_DB_TEST_HOST").unwrap(), proxy.host());
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert!(proxy.fired.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(r["cleanup"], "unknown", "{r}");
    assert_eq!(r["outcome"], "unknown");
    f.target_url = direct;
    let mut c = f.target();
    c.batch_execute("SET lock_timeout='2s'; SELECT pg_advisory_lock(1853124212,245)")
        .unwrap();
    assert!(!c
        .query_one("SELECT to_regclass('app.t') IS NOT NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
    assert!(!c
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='_ntnt')",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
}

fn spawn_apply(f: &Fixture, plan: &Value) -> std::process::Child {
    f.process("migrate")
        .args(["--plan", "upgrade.json"])
        .env("DB_APPROVAL", plan["plan_digest"].as_str().unwrap())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}
fn wait_locked(f: &mut Fixture, count: i64) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let waiting: i64 = f
            .admin
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE usename=$1 AND wait_event_type='Lock'",
                &[&f.target],
            )
            .unwrap()
            .get(0);
        if waiting >= count {
            return;
        }
        assert!(
            std::time::Instant::now() < until,
            "migration did not reach owned lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn result(child: std::process::Child) -> Value {
    let out = child.wait_with_output().unwrap();
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
}
fn upgrade_fixture() -> (Fixture, Value) {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t(id INTEGER PRIMARY KEY);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .batch_execute("INSERT INTO app.t VALUES(17)")
        .unwrap();
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.t ADD COLUMN label TEXT DEFAULT 'once';",
    );
    let plan = f.plan("upgrade.json");
    (f, plan)
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn competing_migrators_serialize_and_execute_captured_bytes_only_once() {
    let (mut f, plan) = upgrade_fixture();
    let mut locker = f.target();
    locker
        .batch_execute("BEGIN; LOCK TABLE app.t IN ACCESS EXCLUSIVE MODE")
        .unwrap();
    let first = spawn_apply(&f, &plan);
    wait_locked(&mut f, 1);
    let second = spawn_apply(&f, &plan);
    wait_locked(&mut f, 2);
    std::fs::remove_file(f.root.path().join("migrations/0002_expand.sql")).unwrap();
    f.sql("0002_expand.sql", "SELECT pg_sleep(30)");
    std::fs::write(f.root.path().join("ntnt.toml"), "invalid manifest").unwrap();
    locker.batch_execute("COMMIT").unwrap();
    let a = result(first);
    let b = result(second);
    assert_eq!(a["outcome"], "pass", "{a}");
    assert_eq!(b["outcome"], "pass", "{b}");
    assert_eq!(a["progress"][1]["state"], "committed");
    assert_eq!(b["progress"][1]["state"], "already_applied");
    assert_eq!(
        f.target()
            .query_one("SELECT label FROM app.t WHERE id=17", &[])
            .unwrap()
            .get::<_, String>(0),
        "once"
    );
    assert_eq!(
        f.target()
            .query_one("SELECT count(*) FROM _ntnt.schema_migrations_v2", &[])
            .unwrap()
            .get::<_, i64>(0),
        2
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn native_and_ownership_timeouts_discard_sessions() {
    for advisory in [false, true] {
        let (f, plan) = upgrade_fixture();
        let mut locker = f.target();
        locker
            .batch_execute(if advisory {
                "SELECT pg_advisory_lock(1853124212,245)"
            } else {
                "BEGIN; LOCK TABLE app.t IN ACCESS EXCLUSIVE MODE"
            })
            .unwrap();
        let start = std::time::Instant::now();
        let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
        assert_ne!(r["outcome"], "pass");
        assert!(start.elapsed() < std::time::Duration::from_secs(8));
        locker.batch_execute("ROLLBACK; SELECT pg_advisory_unlock_all(); SET lock_timeout='2s'; SELECT pg_advisory_lock(1853124212,245); ALTER TABLE app.t ADD COLUMN canary INTEGER").unwrap();
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn cancelled_process_releases_owned_session_and_rerun_inspects_fresh() {
    let (mut f, plan) = upgrade_fixture();
    let mut locker = f.target();
    locker
        .batch_execute("BEGIN; LOCK TABLE app.t IN ACCESS EXCLUSIVE MODE")
        .unwrap();
    let mut child = spawn_apply(&f, &plan);
    wait_locked(&mut f, 1);
    child.kill().unwrap();
    child.wait().unwrap();
    locker.batch_execute("ROLLBACK").unwrap();
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "committed");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn data_snapshot_is_taken_after_native_lock_acquisition() {
    let (mut f, _) = upgrade_fixture();
    std::fs::write(
        f.root.path().join("migrations/0002_expand.toml"),
        "[requires]\npositive='SELECT NOT EXISTS (SELECT 1 FROM app.t WHERE id < 0)'",
    )
    .unwrap();
    std::fs::remove_file(f.root.path().join("upgrade.json")).unwrap();
    let plan = f.plan("upgrade.json");
    let mut writer = f.target();
    writer
        .batch_execute("BEGIN; INSERT INTO app.t VALUES(-1)")
        .unwrap();
    let child = spawn_apply(&f, &plan);
    wait_locked(&mut f, 1);
    writer.batch_execute("COMMIT").unwrap();
    let r = result(child);
    assert!(code(&r, "DB_CHECK_FAILED"), "{r}");
    assert_eq!(r["progress"][1]["state"], "failed");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn expensive_supported_check_times_out_and_rolls_back() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.a(id INTEGER); CREATE TABLE app.b(val INTEGER);",
    );
    let first = f.plan("first.json");
    assert_eq!(
        f.apply("first.json", first["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target().batch_execute("INSERT INTO app.a SELECT generate_series(1,30000); INSERT INTO app.b SELECT generate_series(1,30000)").unwrap();
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.a ADD COLUMN label TEXT;",
    );
    std::fs::write(f.root.path().join("migrations/0002_expand.toml"),"[requires]\nscan='SELECT NOT EXISTS (SELECT 1 FROM app.a WHERE (SELECT count(*) FROM app.b WHERE val > id) < 0)'").unwrap();
    let plan = f.plan("plan.json");
    let start = std::time::Instant::now();
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_TIMEOUT") || code(&r, "DB_POSTGRES"), "{r}");
    assert_eq!(r["assertions"]["executed"], 1);
    assert_eq!(r["assertions"]["outcome"], "fail");
    assert!(start.elapsed() < std::time::Duration::from_secs(12));
    f.target()
        .batch_execute("SET lock_timeout='2s'; ALTER TABLE app.a ADD COLUMN canary INTEGER")
        .unwrap();
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn ledger_mutation_validation_rejects_defaults_checks_triggers_and_legacy() {
    for tamper in [
        "ALTER TABLE _ntnt.schema_migrations_v2 DROP CONSTRAINT schema_migrations_v2_pkey",
        "ALTER TABLE _ntnt.schema_migrations_v2 SET UNLOGGED",
        "ALTER TABLE _ntnt.schema_migrations_v2 ALTER COLUMN state SET DEFAULT 'applied'",
        "ALTER TABLE _ntnt.schema_migrations_v2 ADD CONSTRAINT trap CHECK(pg_try_advisory_lock(7654321)) NOT VALID",
        "CREATE FUNCTION public.trap() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM pg_advisory_lock(7654321); RETURN NEW; END$$; CREATE TRIGGER trap BEFORE INSERT ON _ntnt.schema_migrations_v2 FOR EACH ROW EXECUTE FUNCTION public.trap()",
        "ALTER TABLE _ntnt.schema_migrations_v2 ENABLE ROW LEVEL SECURITY",
        "GRANT INSERT ON _ntnt.schema_migrations_v2 TO PUBLIC",
    ] {
        let (f,plan)=upgrade_fixture();
        f.target().batch_execute(tamper).unwrap();
        let r=f.apply("upgrade.json",plan["plan_digest"].as_str());
        assert_ne!(r["outcome"],"pass","{r}");
        assert!(code(&r,"DB_LEDGER_UNSAFE") || code(&r,"DB_LEDGER_UNKNOWN"),"{r}");
        assert!(!f.target().query_one("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema='app' AND table_name='t' AND column_name='label')",&[]).unwrap().get::<_,bool>(0));
    }
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.t ADD COLUMN label TEXT;",
    );
    let plan = f.plan("legacy.json");
    assert!(code(
        &f.apply("legacy.json", plan["plan_digest"].as_str()),
        "DB_LEDGER_SCOPE"
    ));
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn scoped_history_and_incoming_dependencies_cannot_escape_ownership() {
    let (f, plan) = upgrade_fixture();
    f.target().batch_execute("CREATE SCHEMA external_data; CREATE TABLE external_data.child(id INTEGER REFERENCES app.t(id))").unwrap();
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_EXTERNAL_DEPENDENCY"), "{r}");
    f.sql("0002_expand.sql", "DROP TABLE app.t CASCADE;");
    assert!(code(&f.command("plan"), "DB_REPLAY_UNSUPPORTED"));
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.t DROP COLUMN id CASCADE;",
    );
    assert!(code(&f.command("plan"), "DB_REPLAY_UNSUPPORTED"));
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.t ADD COLUMN label TEXT DEFAULT 'once';",
    );
    let path = f.root.path().join("ntnt.toml");
    let mut manifest: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    let mut other = manifest["databases"]["app"].clone();
    other["schema"] = "other".into();
    manifest["databases"]
        .as_table_mut()
        .unwrap()
        .insert("other".into(), other);
    std::fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
    f.target().batch_execute("CREATE SCHEMA other").unwrap();
    let r = f.command_args("status", &["--database", "other"]);
    assert!(code(&r, "DB_LEDGER_SCOPE"), "{r}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn protected_authority_reference_and_actual_wrong_target_are_required() {
    let (f, plan) = upgrade_fixture();
    let other = Fixture::new();
    std::fs::copy(
        f.root.path().join("upgrade.json"),
        other.root.path().join("plan.json"),
    )
    .unwrap();
    assert!(code(
        &other.apply("plan.json", plan["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    let path = other.root.path().join("ntnt.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("approval_secret = \"DB_APPROVAL\"", "");
    std::fs::write(&path, text).unwrap();
    other.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let fresh = other.plan("fresh.json");
    assert!(code(
        &other.apply("fresh.json", fresh["plan_digest"].as_str()),
        "DB_APPROVAL_REQUIRED"
    ));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn populated_null_duplicate_and_multirow_contracts_reject_before_ddl() {
    for (rows, check) in [
        (
            "(1,'same'),(2,'same')",
            "SELECT NOT EXISTS (SELECT email FROM app.t GROUP BY email HAVING count(*) > 1)",
        ),
        (
            "(1,'preserved'),(2,NULL)",
            "SELECT NOT EXISTS (SELECT 1 FROM app.t WHERE email IS NULL)",
        ),
        ("(1,'preserved'),(2,'other')", "SELECT true FROM app.t"),
    ] {
        let f = Fixture::new();
        f.sql(
            "0001_initial.sql",
            "CREATE TABLE app.t(id INTEGER, email TEXT);",
        );
        let first = f.plan("first.json");
        assert_eq!(
            f.apply("first.json", first["plan_digest"].as_str())["outcome"],
            "pass"
        );
        f.target()
            .batch_execute(&format!("INSERT INTO app.t VALUES {rows}"))
            .unwrap();
        f.sql(
            "0002_unique.sql",
            "ALTER TABLE app.t ADD CONSTRAINT email_unique UNIQUE(email);",
        );
        std::fs::write(
            f.root.path().join("migrations/0002_unique.toml"),
            format!("[requires]\ncheck={check:?}"),
        )
        .unwrap();
        let plan = f.plan("upgrade.json");
        let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
        assert!(code(&r, "DB_CHECK_FAILED"), "{r}");
        assert_eq!(r["assertions"]["executed"], 1);
        assert_eq!(
            f.target()
                .query_one("SELECT count(*) FROM app.t", &[])
                .unwrap()
                .get::<_, i64>(0),
            2
        );
        assert_eq!(
            f.target()
                .query_one("SELECT count(*) FROM _ntnt.schema_migrations_v2", &[])
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        assert!(!f
            .target()
            .query_one("SELECT to_regclass('app.email_unique') IS NOT NULL", &[])
            .unwrap()
            .get::<_, bool>(0));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn adoption_commit_disconnects_reconcile_checkpoints_and_legacy_imports() {
    use db_proxy::{Fault, Proxy};
    use std::io::Write;
    for legacy in [false, true] {
        for fault in [
            Fault::BeforeCommit,
            Fault::AfterCommit,
            Fault::AfterCommitUnavailable,
        ] {
            let mut f = Fixture::new();
            let schema = "CREATE TABLE app.t(id INTEGER PRIMARY KEY,value TEXT);";
            if legacy {
                f.seed_first(schema);
            } else {
                f.sql("0001_initial.sql", schema);
                f.target().batch_execute(schema).unwrap();
            }
            f.target()
                .batch_execute("INSERT INTO app.t VALUES(17,'retained')")
                .unwrap();
            writeln!(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(f.root.path().join("ntnt.toml"))
                    .unwrap(),
                "\n[databases.app.adoption_checks]\ncurrent='SELECT count(*)=1 FROM app.t'"
            )
            .unwrap();
            let plan = f.command_args("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
            assert_eq!(plan["outcome"], "pass", "{plan}");
            let proxy = Proxy::new(fault, 1);
            let direct = f.target_url.clone();
            f.target_url =
                direct.replace(&std::env::var("NTNT_DB_TEST_HOST").unwrap(), proxy.host());
            let result = f.apply("adopt.json", plan["plan_digest"].as_str());
            assert!(proxy.fired.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(result["progress"].as_array().unwrap().len(), 1);
            assert_eq!(result["progress"][0]["migration_sql_executed"], false);
            assert_eq!(
                result["reconciliation"]["outcome"],
                if matches!(fault, Fault::AfterCommitUnavailable) {
                    "unknown"
                } else {
                    "pass"
                },
                "{result}"
            );
            assert_eq!(
                result["progress"][0]["state"],
                match fault {
                    Fault::BeforeCommit => "failed",
                    Fault::AfterCommit => "committed",
                    _ => "unknown",
                },
                "{result}"
            );
            if matches!(fault, Fault::AfterCommit) {
                assert_eq!(result["applied_count"], 0);
                assert_eq!(
                    result[if legacy {
                        "imported_count"
                    } else {
                        "adopted_count"
                    }],
                    1
                );
            }
            f.target_url = direct;
            let recovered = f.apply("adopt.json", plan["plan_digest"].as_str());
            assert_eq!(recovered["outcome"], "pass", "{recovered}");
            assert_eq!(
                recovered["progress"][0]["state"],
                if matches!(fault, Fault::BeforeCommit) {
                    "committed"
                } else if legacy {
                    "already_imported"
                } else {
                    "already_adopted"
                }
            );
            assert_eq!(
                recovered["assertions"]["executed"],
                if matches!(fault, Fault::BeforeCommit) {
                    1
                } else {
                    0
                }
            );
            assert_eq!(
                f.target()
                    .query_one("SELECT count(*) FROM _ntnt.schema_migrations_v2", &[])
                    .unwrap()
                    .get::<_, i64>(0),
                1
            );
            assert_eq!(
                f.target()
                    .query_one("SELECT value FROM app.t WHERE id=17", &[])
                    .unwrap()
                    .get::<_, String>(0),
                "retained"
            );
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn upgrade_reconciliation_preserves_adoption_and_import_counts() {
    use db_proxy::{Fault, Proxy};
    use std::io::Write;
    for legacy in [false, true] {
        for fault in [Fault::BeforeCommit, Fault::AfterCommit] {
            let mut f = Fixture::new();
            let schema = "CREATE TABLE app.t(id INTEGER PRIMARY KEY,value TEXT);";
            if legacy {
                f.seed_first(schema);
            } else {
                f.sql("0001_initial.sql", schema);
                f.target().batch_execute(schema).unwrap();
            }
            f.target()
                .batch_execute("INSERT INTO app.t VALUES(17,'retained')")
                .unwrap();
            writeln!(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(f.root.path().join("ntnt.toml"))
                    .unwrap(),
                "\n[databases.app.adoption_checks]\ncurrent='SELECT count(*)=1 FROM app.t'"
            )
            .unwrap();
            let plan = f.command_args("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
            assert_eq!(plan["outcome"], "pass", "{plan}");
            assert_eq!(
                f.apply("adopt.json", plan["plan_digest"].as_str())["outcome"],
                "pass"
            );
            f.sql(
                "0002_upgrade.sql",
                "ALTER TABLE app.t ADD COLUMN label TEXT DEFAULT 'new';",
            );
            let plan = f.plan("upgrade.json");
            let proxy = Proxy::new(fault, 1);
            let direct = f.target_url.clone();
            f.target_url =
                direct.replace(&std::env::var("NTNT_DB_TEST_HOST").unwrap(), proxy.host());
            let result = f.apply("upgrade.json", plan["plan_digest"].as_str());
            assert!(proxy.fired.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(result["reconciliation"]["outcome"], "pass", "{result}");
            f.target_url = direct;
            let applied = f
                .target()
                .query_one(
                    "SELECT count(*) FROM _ntnt.schema_migrations_v2 WHERE state='applied'",
                    &[],
                )
                .unwrap()
                .get::<_, i64>(0);
            assert_eq!(
                result["reconciliation"]["applied_count"], applied,
                "{result}"
            );
            assert_eq!(
                result["reconciliation"]["adopted_count"],
                if legacy { 0 } else { 1 }
            );
            assert_eq!(
                result["reconciliation"]["imported_count"],
                if legacy { 1 } else { 0 }
            );
            assert_eq!(result["reconciliation"]["covered_count"], applied + 1);
            assert_eq!(result["applied_count"], applied);
            assert_eq!(
                f.apply("upgrade.json", plan["plan_digest"].as_str())["outcome"],
                "pass"
            );
        }
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn public_schema_is_a_managed_schema() {
    // Most applications keep their tables in PostgreSQL's default `public`
    // schema. It must plan, execute, verify and detect drift like any other.
    let f = Fixture::with_schema("public");
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE accounts(id INTEGER PRIMARY KEY, email TEXT NOT NULL);",
    );
    let p = f.plan("plan.json");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert!(f
        .target()
        .query_one("SELECT to_regclass('public.accounts') IS NOT NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
    assert_eq!(f.command("verify")["outcome"], "pass");
    f.sql(
        "0002_name.sql",
        "ALTER TABLE public.accounts ADD COLUMN name TEXT;",
    );
    let p = f.plan("plan2.json");
    let r = f.apply("plan2.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
    f.target()
        .batch_execute("ALTER TABLE public.accounts ADD COLUMN hotfix TEXT")
        .unwrap();
    let drift = f.command("verify");
    assert_eq!(drift["outcome"], "fail", "{drift}");
    assert!(code(&drift, "DB_DRIFT"), "{drift}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn complete_transactional_scripts_replay_and_execute() {
    // Shapes taken from a real application's history: an idempotent DO block,
    // deferred constraints with seed rows, and ON DELETE CASCADE actions.
    let f = Fixture::with_schema("public");
    f.sql(
        "0001_initial.sql",
        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_class WHERE relname = 'plans') THEN
             CREATE TABLE plans(code TEXT PRIMARY KEY, name TEXT NOT NULL);
         END IF; END $$;
         CREATE TABLE tenants(id INTEGER PRIMARY KEY, plan TEXT NOT NULL REFERENCES plans(code) ON DELETE CASCADE DEFERRABLE);
         SET CONSTRAINTS ALL DEFERRED;
         INSERT INTO tenants VALUES (1, 'internal');
         INSERT INTO plans VALUES ('internal', 'Internal; with a semicolon');
         SET CONSTRAINTS ALL IMMEDIATE;",
    );
    let p = f.plan("plan.json");
    assert_eq!(p["outcome"], "pass", "{p}");
    let ops = &p["plan"]["pending"][0]["analysis"]["operations"];
    assert_eq!(ops[0], "do", "{ops}");
    assert!(p["plan"]["pending"][0]["analysis"]["unknowns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u == "SQL_NOT_PARSED"));
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(
        f.target()
            .query_one(
                "SELECT count(*) FROM public.tenants JOIN public.plans ON code = plan",
                &[]
            )
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(f.command("verify")["outcome"], "pass");
}

/// Object shapes from a real application's history (Jarri): routines,
/// triggers, forced RLS policies, a partitioned table with a default
/// partition, partial/expression indexes, a view, an identity sequence and a
/// trusted extension in the managed schema.
const REAL_SCHEMA: &str = "CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE TABLE tenants(id UUID PRIMARY KEY DEFAULT gen_random_uuid(), name TEXT NOT NULL, settings JSONB NOT NULL DEFAULT '{}');
CREATE TABLE audit(id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, tenant_id UUID NOT NULL REFERENCES tenants(id), note TEXT NOT NULL);
CREATE FUNCTION reject_audit_change() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'append only'; END $$;
CREATE TRIGGER audit_append_only BEFORE UPDATE OR DELETE ON audit FOR EACH ROW EXECUTE FUNCTION reject_audit_change();
ALTER TABLE audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_tenant ON audit USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
CREATE TABLE observations(tenant_id UUID NOT NULL, observed_at TIMESTAMPTZ NOT NULL, value DOUBLE PRECISION) PARTITION BY RANGE (observed_at);
CREATE TABLE observations_default PARTITION OF observations DEFAULT;
CREATE INDEX observations_recent ON observations(tenant_id, observed_at DESC);
CREATE INDEX tenants_lower_name ON tenants(lower(name)) WHERE name <> '';
CREATE VIEW tenant_names AS SELECT id, name FROM tenants;";

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn real_application_objects_are_compared_not_left_unknown() {
    let f = Fixture::with_schema("public");
    f.sql("0001_initial.sql", REAL_SCHEMA);
    std::fs::write(
        f.root.path().join("migrations/0001_initial.toml"),
        "[ensures]\nno_tenants = 'SELECT NOT EXISTS (SELECT 1 FROM public.tenants)'\n",
    )
    .unwrap();
    let p = f.plan("plan.json");
    assert_eq!(p["outcome"], "pass", "{p}");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
    let v = f.command("verify");
    assert_eq!(v["outcome"], "pass", "{v}");
    assert_eq!(v["catalog"]["uncovered"], serde_json::json!([]), "{v}");
    // Extension members are compared through the extension's version: their
    // owner is whoever installed it, which differs between scratch and target.
    let digest = &v["catalog"]["objects"]["routine:digest(text, text)"];
    assert_eq!(digest["extension"], "pgcrypto", "{digest}");
    assert!(
        digest["owner"].is_null() && digest["grants"].is_null(),
        "{digest}"
    );
    // Each out-of-band change below is drift on a named object, not "unknown".
    for (change, object) in [
        ("CREATE OR REPLACE FUNCTION public.reject_audit_change() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$", "routine:reject_audit_change()"),
        ("ALTER POLICY audit_tenant ON public.audit USING (true)", "policy:audit.audit_tenant"),
        ("ALTER TABLE public.audit DISABLE TRIGGER audit_append_only", "trigger:audit.audit_append_only"),
        ("CREATE OR REPLACE VIEW public.tenant_names AS SELECT id, upper(name) AS name FROM public.tenants", "view:tenant_names"),
        ("DROP INDEX public.tenants_lower_name; CREATE INDEX tenants_lower_name ON public.tenants(lower(name))", "index:tenants_lower_name"),
    ] {
        f.target().batch_execute(change).unwrap();
        let d = f.command("verify");
        assert_eq!(d["outcome"], "fail", "{change}: {d}");
        assert!(code(&d, "DB_DRIFT"), "{change}: {d}");
        assert!(
            d["drift"]["differences"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x["object"] == object),
            "{change}: {}",
            d["drift"]["differences"]
        );
        break_and_restore(&f);
    }
}

/// Put the applied definitions back so each drift case is isolated.
fn break_and_restore(f: &Fixture) {
    let mut c = f.target();
    c.batch_execute(
        "CREATE OR REPLACE FUNCTION public.reject_audit_change() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'append only'; END $$;
         ALTER POLICY audit_tenant ON public.audit USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
         ALTER TABLE public.audit ENABLE TRIGGER audit_append_only;
         CREATE OR REPLACE VIEW public.tenant_names AS SELECT id, name FROM public.tenants;
         DROP INDEX public.tenants_lower_name; CREATE INDEX tenants_lower_name ON public.tenants(lower(name)) WHERE name <> '';",
    )
    .unwrap();
    let v = f.command("verify");
    assert_eq!(v["outcome"], "pass", "restore failed: {v}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn migration_scripts_run_with_normal_row_security() {
    // Seed rows into a FORCE ROW LEVEL SECURITY table, as a real application's
    // second migration does. Inspection still reads with row_security=off.
    let f = Fixture::with_schema("public");
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE plans(code TEXT PRIMARY KEY);
         ALTER TABLE plans ENABLE ROW LEVEL SECURITY;
         ALTER TABLE plans FORCE ROW LEVEL SECURITY;
         CREATE POLICY plans_all ON plans USING (true) WITH CHECK (true);",
    );
    f.sql("0002_seed.sql", "INSERT INTO plans VALUES ('internal');");
    let p = f.plan("plan.json");
    assert_eq!(p["outcome"], "pass", "{p}");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
    assert_eq!(
        f.target()
            .query_one("SELECT count(*) FROM public.plans", &[])
            .unwrap()
            .get::<_, i64>(0),
        1
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn deferred_constraints_settle_at_each_migration_boundary() {
    // Replay runs a whole history in one rollback-only transaction. Each
    // migration must still end as its commit would: deferred checks fire, so
    // the next migration can ALTER the table.
    let f = Fixture::with_schema("public");
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE plans(code TEXT PRIMARY KEY);
         CREATE TABLE tenants(id INTEGER PRIMARY KEY, plan TEXT NOT NULL REFERENCES plans(code) DEFERRABLE);
         CREATE TABLE notes(id INTEGER PRIMARY KEY, plan TEXT NOT NULL REFERENCES plans(code) DEFERRABLE INITIALLY DEFERRED);",
    );
    f.sql(
        "0002_seed.sql",
        "SET CONSTRAINTS ALL DEFERRED;
         INSERT INTO tenants VALUES (1, 'internal');
         INSERT INTO plans VALUES ('internal');",
    );
    // A later migration in the same replay still sees each constraint's own
    // default: INITIALLY DEFERRED stays deferred without asking for it.
    f.sql(
        "0003_alter.sql",
        "ALTER TABLE tenants ADD COLUMN name TEXT;
         INSERT INTO notes VALUES (1, 'paid');
         INSERT INTO plans VALUES ('paid');",
    );
    let p = f.plan("plan.json");
    assert_eq!(p["outcome"], "pass", "{p}");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    // A deferred violation is still the failing migration's error.
    f.sql(
        "0004_bad.sql",
        "SET CONSTRAINTS ALL DEFERRED; INSERT INTO tenants VALUES (2, 'missing', 'x');",
    );
    let bad = f.command("plan");
    assert!(code(&bad, "DB_POSTGRES"), "{bad}");
    assert!(
        bad["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["source"] == "0004_bad.sql"),
        "{bad}"
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn scratch_matches_a_target_role_that_bypasses_row_security() {
    // Many applications migrate as a role that bypasses row-level security, so
    // seed rows are not filtered by tenant policies. Scratch must match that,
    // and only that: it may bypass RLS only when the target role does.
    let mut f = Fixture::with_schema("public");
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE tenants(id INTEGER PRIMARY KEY);
         ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
         ALTER TABLE tenants FORCE ROW LEVEL SECURITY;
         CREATE POLICY tenant_self ON tenants USING (id = NULLIF(current_setting('app.tenant_id', true), '')::integer)
           WITH CHECK (id = NULLIF(current_setting('app.tenant_id', true), '')::integer);
         INSERT INTO tenants VALUES (1);",
    );
    let r = f.command("plan");
    assert!(
        code(&r, "DB_POSTGRES"),
        "policy applies without bypass: {r}"
    );
    f.admin
        .batch_execute(&format!("ALTER ROLE {} BYPASSRLS", f.scratch))
        .unwrap();
    let r = f.command("plan");
    assert!(
        code(&r, "DB_SCRATCH_NOT_EMPTY"),
        "scratch may not exceed target: {r}"
    );
    f.admin
        .batch_execute(&format!("ALTER ROLE {} BYPASSRLS", f.target))
        .unwrap();
    let p = f.plan("plan.json");
    assert_eq!(p["outcome"], "pass", "{p}");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}
