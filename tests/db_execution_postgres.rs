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
