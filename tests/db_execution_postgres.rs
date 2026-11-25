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
    spawn_plan(f, "upgrade.json", plan)
}
fn spawn_plan(f: &Fixture, path: &str, plan: &Value) -> std::process::Child {
    f.process("migrate")
        .args(["--plan", path])
        .env("DB_APPROVAL", plan["plan_digest"].as_str().unwrap())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}
/// Hang guard for waiting on `migrate` or on a PostgreSQL state: only a bug
/// or a hang reaches it. It stays below the 90-second default statement
/// limit, so a lowered limit that is not applied fails as a hang.
const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(60);
/// A deliberate minimum hold of a lock a migration waits for: longer than
/// the 2-second low limits below and the 5-second session, client and
/// migrator-wait defaults that configured limits replace. Every limit such
/// a test must not hit is `raised`, ten times this hold.
const PAST_THE_DEFAULTS: std::time::Duration = std::time::Duration::from_secs(6);
/// `field = ` ten times PAST_THE_DEFAULTS, for a `[databases.app.limits]` table.
fn raised(field: &str) -> String {
    format!("{field} = {}", 10 * PAST_THE_DEFAULTS.as_secs())
}
/// Poll `probe` until it returns a value; only a bug or a hang reaches
/// HANG_GUARD.
fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let until = std::time::Instant::now() + HANG_GUARD;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(
            std::time::Instant::now() < until,
            "{what} not seen within {HANG_GUARD:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
fn wait_locked(f: &mut Fixture, count: i64) {
    wait_for("migration waiting for a lock", || {
        let waiting: i64 = f
            .admin
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE usename=$1 AND wait_event_type='Lock'",
                &[&f.target],
            )
            .unwrap()
            .get(0);
        (waiting >= count).then_some(())
    });
}
/// The one target session waiting for a lock: its pid, the start of its
/// transaction and its query.
fn lock_waiter(f: &mut Fixture) -> (i32, Option<String>, String) {
    wait_for("a target session waiting for a lock", || {
        let rows = f
            .admin
            .query(
                "SELECT pid, xact_start::text, query FROM pg_stat_activity WHERE usename=$1 AND wait_event_type='Lock'",
                &[&f.target],
            )
            .unwrap();
        assert!(rows.len() <= 1, "{} sessions wait for a lock", rows.len());
        rows.first().map(|row| (row.get(0), row.get(1), row.get(2)))
    })
}
fn result(child: std::process::Child) -> Value {
    let out = child.wait_with_output().unwrap();
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
}
/// `result`, but kill the process and fail if it is still running after
/// HANG_GUARD.
fn result_within(mut child: std::process::Child) -> Value {
    let until = std::time::Instant::now() + HANG_GUARD;
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= until {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("migrate still running after {HANG_GUARD:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    result(child)
}
fn upgrade_fixture() -> (Fixture, Value) {
    upgrade_fixture_with_limits(None)
}
/// `upgrade_fixture`, planned with a `[databases.app.limits]` table.
fn upgrade_fixture_with_limits(limits: Option<&str>) -> (Fixture, Value) {
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
    if let Some(limits) = limits {
        set_limits(&f, limits);
    }
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

/// #430 (cli-17): checks read with row_security=off, so PostgreSQL refuses
/// a check's read of a table whose row-level security applies to the
/// migration role: FORCE ROW LEVEL SECURITY on a table the documented
/// unprivileged owner owns. The plan warns, naming each check and table, and
/// migrate refuses the check by name instead of failing with a bare SQLSTATE
/// 42501. A view the role owns reads as the role, so a check through it is
/// caught too. Once the role has BYPASSRLS, a new plan has no warning and
/// migrate passes.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_on_forced_row_security_tables_are_flagged_and_named() {
    let mut f = Fixture::new();
    f.sql(
        "0001_tenants.sql",
        "CREATE TABLE app.tenants (id BIGINT PRIMARY KEY, slug TEXT NOT NULL);
         ALTER TABLE app.tenants ENABLE ROW LEVEL SECURITY;
         ALTER TABLE app.tenants FORCE ROW LEVEL SECURITY;
         CREATE POLICY tenant_all ON app.tenants USING (true) WITH CHECK (true);
         INSERT INTO app.tenants VALUES (1, 'internal');
         CREATE VIEW app.tenant_slugs AS SELECT slug FROM app.tenants;
         CREATE TABLE app.plain (id BIGINT PRIMARY KEY);",
    );
    f.sql(
        "0002_names.sql",
        "ALTER TABLE app.tenants ADD COLUMN name TEXT;",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_names.toml"),
        "[requires]\nslugs_via_view = \"SELECT EXISTS (SELECT 1 FROM app.tenant_slugs)\"\nplain_ok = \"SELECT NOT EXISTS (SELECT 1 FROM app.plain)\"\n[ensures]\nslugs_present = \"SELECT EXISTS (SELECT 1 FROM app.tenants WHERE slug = 'internal')\"\n",
    )
    .unwrap();
    let p = f.plan("plan.json");
    let warnings: Vec<_> = p["plan"]["check_warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("plan flags the checks: {p}"))
        .iter()
        .map(|w| {
            assert_eq!(w["code"], "CHECK_ROW_SECURITY", "{w}");
            assert_eq!(w["source"], "0002_names.sql", "{w}");
            assert_eq!(w["relation"], "app.tenants", "{w}");
            assert_eq!(w["forced"], true, "{w}");
            let message = w["message"].as_str().unwrap();
            assert!(message.contains("BYPASSRLS"), "{w}");
            (
                w["phase"].as_str().unwrap().to_owned(),
                w["check"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        warnings,
        [
            ("requires".to_owned(), "slugs_via_view".to_owned()),
            ("ensures".to_owned(), "slugs_present".to_owned())
        ],
        "{p}"
    );
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "fail", "{r}");
    let d = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_CHECK_ROW_SECURITY")
        .unwrap_or_else(|| panic!("named refusal, not a bare SQLSTATE: {r}"));
    let message = d["message"].as_str().unwrap();
    // Reached through a view, the table is named once PostgreSQL refuses the
    // read: a view may name a table it does not read.
    assert!(
        message.starts_with("Check slugs_via_view ([requires]) reads app.tenants (through a view; FORCE ROW LEVEL SECURITY)")
            && message.contains(&f.target)
            && message.ends_with("PostgreSQL refused this check's read."),
        "{d}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains(&format!("ALTER ROLE {} BYPASSRLS", f.target)),
        "{d}"
    );
    assert_eq!(d["source"], "0002_names.sql", "{d}");
    assert_eq!(d["objects"], serde_json::json!(["relation:tenants"]), "{d}");
    // Checks run in name order: plain_ok passed before the refused one.
    assert_eq!(
        r["assertions"]["checks"],
        serde_json::json!([
            {"source":"0002_names.sql","name":"plain_ok","phase":"requires","outcome":"pass"},
            {"source":"0002_names.sql","name":"slugs_via_view","phase":"requires","outcome":"fail"}
        ]),
        "{r}"
    );
    assert_eq!(r["assertions"]["executed"], 2, "{r}");
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    let added: bool = f
        .target()
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_schema='app' AND table_name='tenants' AND column_name='name')",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(!added, "the refused migration was rolled back");
    // A check that names the table is refused before it runs and is not
    // counted as executed.
    let names_toml =
        std::fs::read_to_string(f.root.path().join("migrations/0002_names.toml")).unwrap();
    std::fs::write(
        f.root.path().join("migrations/0002_names.toml"),
        "[requires]\nslugs_present = \"SELECT EXISTS (SELECT 1 FROM app.tenants WHERE slug = 'internal')\"\n",
    )
    .unwrap();
    let p = f.plan("plan_direct.json");
    let r = f.apply("plan_direct.json", p["plan_digest"].as_str());
    let d = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_CHECK_ROW_SECURITY")
        .unwrap_or_else(|| panic!("named refusal: {r}"));
    let message = d["message"].as_str().unwrap();
    assert!(
        message.starts_with(
            "Check slugs_present ([requires]) reads app.tenants (FORCE ROW LEVEL SECURITY)"
        ) && message.ends_with("The check did not run."),
        "{d}"
    );
    assert_eq!(d["source"], "0002_names.sql", "{d}");
    assert_eq!(r["assertions"]["executed"], 0, "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    std::fs::write(f.root.path().join("migrations/0002_names.toml"), names_toml).unwrap();
    // The documented remedy: the migration role bypasses row-level security.
    // Its attributes are part of the plan's binding, so plan again.
    f.admin
        .batch_execute(&format!("ALTER ROLE {} BYPASSRLS", f.target))
        .unwrap();
    let p = f.plan("plan2.json");
    assert!(p["plan"].get("check_warnings").is_none(), "{p}");
    let r = f.apply("plan2.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 3, "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

/// #456 and #430 together: one `plan.check_warnings` for both. A check on a
/// FORCE ROW LEVEL SECURITY table that compares a citext column gets both
/// warnings, the operator's first, as migrate meets them; a second check
/// on the table gets only the row-security one. migrate refuses the first
/// check before any check of the phase runs and rolls the migration back.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn a_check_with_several_hazards_gets_each_warning_and_is_refused() {
    let f = Fixture::new();
    f.sql(
        "0001_users.sql",
        "CREATE EXTENSION citext WITH SCHEMA app;
         CREATE TABLE app.users (id BIGINT PRIMARY KEY, email app.citext NOT NULL);
         ALTER TABLE app.users ENABLE ROW LEVEL SECURITY;
         ALTER TABLE app.users FORCE ROW LEVEL SECURITY;
         CREATE POLICY users_all ON app.users USING (true) WITH CHECK (true);
         INSERT INTO app.users VALUES (1, 'Admin@example.com');",
    );
    std::fs::write(
        f.root.path().join("migrations/0001_users.toml"),
        "[ensures]\nadmin_present = \"SELECT EXISTS (SELECT 1 FROM app.users WHERE email = 'admin@example.com')\"\none_user = \"SELECT (SELECT count(*) FROM app.users) = 1\"\n",
    )
    .unwrap();
    let p = f.plan("plan.json");
    let warnings = p["plan"]["check_warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("plan flags the checks: {p}"));
    let found: Vec<(&str, &str)> = warnings
        .iter()
        .map(|w| {
            assert_eq!(w["source"], "0001_users.sql", "{w}");
            assert_eq!(w["phase"], "ensures", "{w}");
            assert_eq!(w["relation"], "app.users", "{w}");
            (w["check"].as_str().unwrap(), w["code"].as_str().unwrap())
        })
        .collect();
    assert_eq!(
        found,
        [
            ("admin_present", "CHECK_OPERATOR_MISMATCH"),
            ("admin_present", "CHECK_ROW_SECURITY"),
            ("one_user", "CHECK_ROW_SECURITY"),
        ],
        "{p}"
    );
    assert!(warnings[0].get("forced").is_none(), "{p}");
    assert_eq!(warnings[1]["forced"], true, "{p}");
    assert!(
        warnings[0]["message"]
            .as_str()
            .unwrap()
            .contains("app.=(app.citext,app.citext)")
            && warnings[1]["message"]
                .as_str()
                .unwrap()
                .contains("BYPASSRLS"),
        "{p}"
    );
    // The human plan prints each warning under its migration.
    let human = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "plan", "--target", "test"])
        .env("DB_TARGET", &f.target_url)
        .env("DB_SCRATCH", &f.scratch_url)
        .env_remove("DB_APPROVAL")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(f.root.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&human.stdout);
    for line in [
        "Warning \"CHECK_OPERATOR_MISMATCH\" on \"app.users\" (\"ensures\" check \"admin_present\")",
        "Warning \"CHECK_ROW_SECURITY\" on \"app.users\" (\"ensures\" check \"admin_present\")",
        "Warning \"CHECK_ROW_SECURITY\" on \"app.users\" (\"ensures\" check \"one_user\")",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "fail", "{r}");
    let d = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_CHECK_UNSUPPORTED")
        .unwrap_or_else(|| panic!("refused before any check runs: {r}"));
    let message = d["message"].as_str().unwrap();
    assert!(
        message.starts_with("Check admin_present ([ensures]) compares column email of app.users (type app.citext) with =")
            && message.ends_with("No check of this phase ran."),
        "{d}"
    );
    assert_eq!(d["source"], "0001_users.sql", "{d}");
    assert_eq!(r["assertions"]["executed"].as_u64().unwrap_or(0), 0, "{r}");
    assert_eq!(r["progress"][0]["state"], "failed", "{r}");
    let created: bool = f
        .target()
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='app' AND c.relname='users')",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(!created, "the refused migration was rolled back");
}

/// #430: a superuser session in the fixture database `db`.
/// #457 review: only PostgreSQL's own row-security refusal is explained as
/// one. A check reads a view that names a FORCE ROW LEVEL SECURITY table
/// as a `regclass` constant, without reading it, and a table the migration
/// role may lock (UPDATE) but not read (SELECT revoked): its SQLSTATE 42501
/// is a missing privilege, reported as the server's.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_keep_other_permission_failures_apart_from_row_security() {
    let f = Fixture::new();
    f.sql(
        "0001_tables.sql",
        "CREATE TABLE app.secret (id BIGINT PRIMARY KEY);
         ALTER TABLE app.secret ENABLE ROW LEVEL SECURITY;
         ALTER TABLE app.secret FORCE ROW LEVEL SECURITY;
         CREATE VIEW app.names AS SELECT 'app.secret'::regclass AS r;
         CREATE TABLE app.hidden (id BIGINT PRIMARY KEY);
         REVOKE SELECT ON app.hidden FROM CURRENT_USER;
         CREATE VIEW app.reader AS SELECT n.r, h.id FROM app.names n, app.hidden h;",
    );
    f.sql(
        "0002_more.sql",
        "CREATE TABLE app.more (id BIGINT PRIMARY KEY);",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_more.toml"),
        "[requires]\nreader_empty = \"SELECT NOT EXISTS (SELECT 1 FROM app.reader)\"\n",
    )
    .unwrap();
    let p = f.plan("plan.json");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "fail", "{r}");
    let codes: Vec<&str> = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert!(!codes.contains(&"DB_CHECK_ROW_SECURITY"), "{r}");
    let d = diagnostic(&r, "DB_POSTGRES");
    assert!(
        d["message"].as_str().unwrap().contains("(SQLSTATE 42501)"),
        "{d}"
    );
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
}

/// #458 review: a domain whose constraint uses an operator outside the
/// managed scope that inspection does not compare, here pg_trgm's `%` in
/// `public` (a declared extension), is uncovered, as one whose constraint
/// calls such a routine is: pg_depend names the operator, not its function.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn domains_using_uninspected_operators_are_uncovered() {
    let f = Fixture::new();
    for db in [&f.target, &f.scratch] {
        superuser_in(db)
            .batch_execute("CREATE EXTENSION pg_trgm SCHEMA public")
            .unwrap();
    }
    declare_extensions(&f, Some("[\"pg_trgm\"]"));
    f.sql(
        "0001_words.sql",
        "CREATE DOMAIN app.word AS text CHECK (VALUE OPERATOR(public.%) 'word' OR VALUE <> '');
         CREATE TABLE app.words (w app.word);",
    );
    let r = f.command_args("plan", &["--out", "plan.json"]);
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_COVERAGE_INCOMPLETE");
    assert!(
        d["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o == "type:word"),
        "{d}"
    );
}

fn superuser_in(db: &str) -> postgres::Client {
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    config.dbname(db);
    config.connect(postgres::NoTls).unwrap()
}

/// `f`'s manifest with `[databases.app] extensions = <list>`, or without the
/// key when `list` is `None`.
fn declare_extensions(f: &Fixture, list: Option<&str>) {
    let path = f.root.path().join("ntnt.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let text: String = text
        .lines()
        .filter(|line| !line.starts_with("extensions = "))
        .map(|line| format!("{line}\n"))
        .collect();
    let text = match list {
        Some(list) => text.replacen(
            "approval_secret = \"DB_APPROVAL\"\n",
            &format!("approval_secret = \"DB_APPROVAL\"\nextensions = {list}\n"),
            1,
        ),
        None => text,
    };
    std::fs::write(path, text).unwrap();
}

fn diagnostic<'a>(r: &'a Value, code: &str) -> &'a Value {
    r["diagnostics"]
        .as_array()
        .and_then(|ds| ds.iter().find(|d| d["code"] == code))
        .unwrap_or_else(|| panic!("{code}: {r}"))
}

/// The untrusted extensions of #430, installed by a superuser in `public`.
const DECLARED: &str = "CREATE EXTENSION postgres_fdw SCHEMA public; CREATE EXTENSION pg_stat_statements SCHEMA public; CREATE EXTENSION pageinspect SCHEMA public;";

/// #430: an extension a superuser installs in the target and the scratch
/// database, declared in the manifest, is compared by version and schema
/// wherever it is installed, in `public` too, and a migration may keep
/// `CREATE EXTENSION IF NOT EXISTS` for it, which does nothing. Undeclared,
/// a DBA's monitoring extension in `public` still makes the catalog
/// uncovered, and the diagnostic says to declare it. With `public` as the
/// managed schema, the extensions' members there are covered by their
/// versions, the relations of pg_stat_statements included.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn declared_superuser_installed_extensions_plan_migrate_and_verify() {
    for schema in ["app", "public"] {
        let f = Fixture::with_schema(schema);
        f.sql(
            "0001_fdw.sql",
            &format!("CREATE EXTENSION IF NOT EXISTS postgres_fdw;\nCREATE TABLE {schema}.accounts (id BIGINT PRIMARY KEY);\n"),
        );
        // A DBA installed pg_stat_statements on the target only, and nothing
        // declares it: still uncovered, now with the remedy.
        superuser_in(&f.target)
            .batch_execute("CREATE EXTENSION pg_stat_statements SCHEMA public")
            .unwrap();
        let r = f.command("plan");
        if schema == "app" {
            let d = diagnostic(&r, "DB_COVERAGE_INCOMPLETE");
            assert_eq!(
                d["objects"],
                serde_json::json!(["extension:pg_stat_statements"]),
                "{d}"
            );
            assert!(
                d["message"].as_str().unwrap().contains(
                    "Extension pg_stat_statements is installed outside the managed scope. If a superuser installed it, declare it in [databases.app] extensions"
                ),
                "{d}"
            );
        } else {
            // In the managed schema, its objects look like a history.
            diagnostic(&r, "DB_UNRECOGNIZED_HISTORY");
        }
        // Declared and installed in both databases, by a superuser.
        superuser_in(&f.target)
            .batch_execute(
                "CREATE EXTENSION postgres_fdw SCHEMA public; CREATE EXTENSION pageinspect SCHEMA public;",
            )
            .unwrap();
        superuser_in(&f.scratch).batch_execute(DECLARED).unwrap();
        declare_extensions(
            &f,
            Some(r#"["postgres_fdw", "pg_stat_statements", "pageinspect"]"#),
        );
        let p = f.plan("plan.json");
        assert!(
            p["catalog"]["uncovered"].as_array().unwrap().is_empty(),
            "{schema}: {p}"
        );
        let fdw = &p["catalog"]["objects"]["extension:postgres_fdw"];
        assert_eq!(fdw["schema"], "public", "{schema}: {p}");
        assert!(fdw["version"].is_string(), "{schema}: {p}");
        let r = f.apply("plan.json", p["plan_digest"].as_str());
        assert_eq!(r["outcome"], "pass", "{schema}: {r}");
        let v = f.command("verify");
        assert_eq!(v["outcome"], "pass", "{schema}: {v}");
        if schema == "public" {
            // Members in the managed schema are inspected and covered.
            assert!(
                v["catalog"]["objects"]["relation:pg_stat_statements"].is_object(),
                "{v}"
            );
        }
    }
}

/// #430: a declared extension owns what depends on its members internally,
/// not just its members. tablefunc's composite types
/// (`tablefunc_crosstab_2`, ...) and dblink's `dblink_pkey_results` are
/// members, but their relations are not. Installed in `public` of both
/// databases and declared, they leave scratch acceptable, and replay does
/// not take them for the history's work: the plan migrates and verifies.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn declared_extensions_own_what_depends_on_their_members() {
    for schema in ["app", "public"] {
        let f = Fixture::with_schema(schema);
        f.sql(
            "0001_t.sql",
            &format!("CREATE TABLE {schema}.t (id BIGINT PRIMARY KEY);\n"),
        );
        for db in [&f.target, &f.scratch] {
            superuser_in(db)
                .batch_execute(
                    "CREATE EXTENSION tablefunc SCHEMA public; CREATE EXTENSION dblink SCHEMA public;",
                )
                .unwrap();
        }
        declare_extensions(&f, Some(r#"["tablefunc", "dblink"]"#));
        let p = f.plan("plan.json");
        assert!(
            p["catalog"]["uncovered"].as_array().unwrap().is_empty(),
            "{schema}: {p}"
        );
        let r = f.apply("plan.json", p["plan_digest"].as_str());
        assert_eq!(r["outcome"], "pass", "{schema}: {r}");
        let v = f.command("verify");
        assert_eq!(v["outcome"], "pass", "{schema}: {v}");
    }
}

/// #430: a declared extension's implicit casts do not block checks, as they
/// do not block DDL: citext's, in `public` of both databases, with a check on
/// an ordinary managed table.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn declared_extensions_casts_do_not_block_checks() {
    let f = Fixture::new();
    for db in [&f.target, &f.scratch] {
        superuser_in(db)
            .batch_execute("CREATE EXTENSION citext SCHEMA public")
            .unwrap();
    }
    declare_extensions(&f, Some(r#"["citext"]"#));
    f.sql(
        "0001_accounts.sql",
        "CREATE TABLE app.accounts (id BIGINT PRIMARY KEY, name TEXT);\n",
    );
    let p = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", p["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql(
        "0002_active.sql",
        "ALTER TABLE app.accounts ADD COLUMN active BOOLEAN NOT NULL DEFAULT true;",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_active.toml"),
        "[ensures]\nactive = \"SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE NOT active)\"\n",
    )
    .unwrap();
    let p = f.plan("checked.json");
    let r = f.apply("checked.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

/// #430: with `public` managed, what a declared extension installs there is
/// inspected, but its owner and grants are its installer's and are not
/// compared, as for an extension's routines and types. Another superuser may
/// install it in scratch.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn declared_extensions_installed_by_another_superuser_match() {
    let f = Fixture::with_schema("public");
    f.sql(
        "0001_t.sql",
        "CREATE TABLE public.t (id BIGINT PRIMARY KEY);\n",
    );
    let installer = format!("{}_inst", f.scratch);
    let install = "CREATE EXTENSION pg_stat_statements SCHEMA public; CREATE EXTENSION tablefunc SCHEMA public;";
    superuser_in(&f.target).batch_execute(install).unwrap();
    let mut scratch = superuser_in(&f.scratch);
    scratch
        .batch_execute(&format!(
            "CREATE ROLE {installer} SUPERUSER; SET ROLE {installer}; {install} RESET ROLE;"
        ))
        .unwrap();
    declare_extensions(&f, Some(r#"["pg_stat_statements", "tablefunc"]"#));
    let p = f.plan("plan.json");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    let v = f.command("verify");
    assert_eq!(v["outcome"], "pass", "{v}");
    assert!(
        v["catalog"]["objects"]["relation:pg_stat_statements"].is_object(),
        "{v}"
    );
    scratch
        .batch_execute(&format!(
            "REASSIGN OWNED BY {installer} TO CURRENT_USER; DROP OWNED BY {installer}; DROP ROLE {installer};"
        ))
        .unwrap();
}

/// #430: a declared extension must be in the scratch database as on the
/// target. A version that differs is drift on the extension, by name; one
/// missing from scratch, or installed where NTNT keeps its ledger, is
/// refused by name; and on the target, one installed in `_ntnt` still fails
/// ledger validation.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn declared_extensions_must_match_between_target_and_scratch() {
    let f = Fixture::new();
    f.sql(
        "0001_t.sql",
        "CREATE TABLE app.t (id BIGINT PRIMARY KEY);\n",
    );
    declare_extensions(&f, Some(r#"["pg_stat_statements"]"#));
    let mut target = superuser_in(&f.target);
    let mut scratch = superuser_in(&f.scratch);
    target
        .batch_execute("CREATE EXTENSION pg_stat_statements SCHEMA public")
        .unwrap();
    // Missing from scratch: refused by name.
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_MISMATCH");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .starts_with("Scratch lacks the declared extension pg_stat_statements."),
        "{d}"
    );
    // An older version in scratch: drift on the extension.
    let older: String = scratch
        .query_one(
            "SELECT version FROM pg_available_extension_versions WHERE name='pg_stat_statements' AND version <> (SELECT default_version FROM pg_available_extensions WHERE name='pg_stat_statements') AND string_to_array(version,'.')::int[] >= '{1,8}' ORDER BY string_to_array(version,'.')::int[] DESC LIMIT 1",
            &[],
        )
        .unwrap()
        .get(0);
    scratch
        .batch_execute(&format!(
            "CREATE EXTENSION pg_stat_statements VERSION '{older}' SCHEMA public"
        ))
        .unwrap();
    let r = f.command("plan");
    assert_eq!(diagnostic(&r, "DB_DRIFT")["code"], "DB_DRIFT");
    assert!(
        r["drift"]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["object"] == "extension:pg_stat_statements"),
        "{r}"
    );
    // The same version in another schema: drift on the extension too.
    scratch
        .batch_execute("DROP EXTENSION pg_stat_statements; CREATE SCHEMA monitoring; CREATE EXTENSION pg_stat_statements SCHEMA monitoring")
        .unwrap();
    let r = f.command("plan");
    assert!(
        r["drift"]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["object"] == "extension:pg_stat_statements"),
        "{r}"
    );
    // In scratch's _ntnt: refused by name.
    scratch
        .batch_execute("DROP EXTENSION pg_stat_statements; DROP SCHEMA monitoring; CREATE SCHEMA _ntnt; CREATE EXTENSION pg_stat_statements SCHEMA _ntnt")
        .unwrap();
    let r = f.command("plan");
    assert!(
        diagnostic(&r, "DB_SCRATCH_MISMATCH")["message"]
            .as_str()
            .unwrap()
            .starts_with(
                "The declared extension pg_stat_statements is installed in schema _ntnt in scratch"
            ),
        "{r}"
    );
    scratch
        .batch_execute("DROP EXTENSION pg_stat_statements; DROP SCHEMA _ntnt; CREATE EXTENSION pg_stat_statements SCHEMA public")
        .unwrap();
    let p = f.plan("plan.json");
    assert_eq!(
        f.apply("plan.json", p["plan_digest"].as_str())["outcome"],
        "pass"
    );
    // On the target, a declared extension in _ntnt fails ledger validation.
    target
        .batch_execute("CREATE EXTENSION pageinspect SCHEMA _ntnt")
        .unwrap();
    declare_extensions(&f, Some(r#"["pg_stat_statements", "pageinspect"]"#));
    scratch
        .batch_execute("CREATE SCHEMA ext; CREATE EXTENSION pageinspect SCHEMA ext")
        .unwrap();
    let r = f.command("status");
    assert!(
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "DB_LEDGER_UNKNOWN"),
        "{r}"
    );
    // Declared, but where NTNT keeps its ledger: never covered.
    assert_eq!(
        r["catalog"]["uncovered"],
        serde_json::json!(["extension:pageinspect"]),
        "{r}"
    );
}

/// #430: scratch's `public`, and a declared extension's own schema, may hold
/// only what the declared extensions own: members, and what depends on a
/// member internally or automatically (array and row types, indexes).
/// Anything else there is refused before replay: a table, an operator over
/// builtin functions in `public`, a sequence in the extension's schema.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn scratch_holds_only_what_declared_extensions_own() {
    let f = Fixture::new();
    f.sql(
        "0001_t.sql",
        "CREATE TABLE app.t (id BIGINT PRIMARY KEY);\n",
    );
    declare_extensions(&f, Some(r#"["pg_stat_statements", "pageinspect"]"#));
    let setup = "CREATE EXTENSION pg_stat_statements SCHEMA public; CREATE SCHEMA ext; CREATE EXTENSION pageinspect SCHEMA ext;";
    superuser_in(&f.target).batch_execute(setup).unwrap();
    let mut scratch = superuser_in(&f.scratch);
    scratch.batch_execute(setup).unwrap();
    f.plan("before.json");
    for (stray, undo) in [
        ("CREATE TABLE public.stray (id int)", "DROP TABLE public.stray"),
        (
            "CREATE OPERATOR public.=== (LEFTARG = int, RIGHTARG = int, FUNCTION = pg_catalog.int4eq)",
            "DROP OPERATOR public.=== (int, int)",
        ),
        // A sequence has no row type: only the relation check sees it.
        ("CREATE SEQUENCE ext.stray", "DROP SEQUENCE ext.stray"),
        // Every kind of object counts in a declared extension's schema too.
        (
            "CREATE OPERATOR ext.=== (LEFTARG = int, RIGHTARG = int, FUNCTION = pg_catalog.int4eq)",
            "DROP OPERATOR ext.=== (int, int)",
        ),
    ] {
        scratch.batch_execute(stray).unwrap();
        let r = f.command("plan");
        assert_eq!(
            diagnostic(&r, "DB_SCRATCH_NOT_EMPTY")["code"],
            "DB_SCRATCH_NOT_EMPTY",
            "{stray}"
        );
        scratch.batch_execute(undo).unwrap();
    }
    let p = f.plan("plan.json");
    assert_eq!(
        f.apply("plan.json", p["plan_digest"].as_str())["outcome"],
        "pass"
    );
}

/// #430 (pg-09): the unprivileged roles NTNT requires can create only
/// trusted extensions. Creating an untrusted one fails with a diagnostic
/// that names the statement and the extension and explains trusted and
/// untrusted extensions, instead of a bare SQLSTATE 42501. One created in a
/// DO block is still explained, without a name.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn untrusted_extensions_fail_with_a_named_privilege_diagnostic() {
    let f = Fixture::new();
    let refusal = |r: &Value| -> Value {
        assert_eq!(r["outcome"], "unknown", "{r}");
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["code"] == "DB_EXTENSION_PRIVILEGE")
            .unwrap_or_else(|| panic!("a named privilege diagnostic: {r}"))
            .clone()
    };
    f.sql(
        "0001_fdw.sql",
        "CREATE TABLE app.t (id int PRIMARY KEY);\nCREATE EXTENSION IF NOT EXISTS postgres_fdw SCHEMA app;",
    );
    let d = refusal(&f.command("plan"));
    let message = d["message"].as_str().unwrap();
    assert!(
        message.starts_with("CREATE EXTENSION postgres_fdw (statement 2) failed: PostgreSQL refused to create an untrusted extension for a role that is not a superuser (SQLSTATE 42501)."),
        "{d}"
    );
    assert!(message.contains("only trusted extensions"), "{d}");
    assert_eq!(d["source"], "0001_fdw.sql", "{d}");
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("trusted extension"),
        "{d}"
    );
    f.sql(
        "0001_fdw.sql",
        "DO $$ BEGIN CREATE EXTENSION postgres_fdw; END $$;\nCREATE EXTENSION pgcrypto SCHEMA app;",
    );
    let d = refusal(&f.command("plan"));
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .starts_with("CREATE EXTENSION (or ALTER EXTENSION ... UPDATE) failed for lack of privilege (SQLSTATE 42501)"),
        "{d}"
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn deferred_constraints_settle_at_each_migration_boundary() {
    // Each migration ends as its commit would, in replay and on the target:
    // deferred checks fire, so the next migration can ALTER the table.
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
    // A later migration still sees each constraint's own default: INITIALLY
    // DEFERRED stays deferred without asking for it.
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

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn migrations_do_not_lock_unrelated_tables() {
    // An application writing to another table must not block (or be blocked
    // by) a migration that never touches that table.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t(id INTEGER); CREATE TABLE app.busy(id INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    let mut writer = f.target();
    writer
        .batch_execute("BEGIN; INSERT INTO app.busy VALUES (1)")
        .unwrap();
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.t ADD COLUMN label TEXT;",
    );
    let plan = f.plan("expand.json");
    let r = f.apply("expand.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    writer.batch_execute("COMMIT").unwrap();
}

/// Hold `mode` on `app.t` while `migrate` runs the upgrade; the statement
/// that waits for it contains `waits_in`. Keep the lock until that
/// statement's transaction has ended, which while the lock is held only the
/// 1-second lock timeout (or a failure) can do, then release it and check
/// that the whole migration transaction was retried and committed.
fn retries_through_a_brief_lock(
    f: &mut Fixture,
    plan: &Value,
    mode: &str,
    waits_in: &str,
) -> Value {
    let mut locker = f.target();
    locker
        .batch_execute(&format!("BEGIN; LOCK TABLE app.t IN {mode} MODE"))
        .unwrap();
    let child = spawn_apply(f, plan);
    let (pid, transaction, query) = lock_waiter(f);
    assert!(query.contains(waits_in), "{query}");
    wait_for("the end of the waiting transaction", || {
        let current: Option<Option<String>> = f
            .admin
            .query_opt(
                "SELECT xact_start::text FROM pg_stat_activity WHERE pid=$1",
                &[&pid],
            )
            .unwrap()
            .map(|row| row.get(0));
        (current != Some(transaction.clone())).then_some(())
    });
    locker.batch_execute("COMMIT").unwrap();
    let r = result_within(child);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "committed", "{r}");
    assert!(
        r["progress"][1]["lock_retries"].as_u64().unwrap_or(0) >= 1,
        "{r}"
    );
    r
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn brief_lock_conflicts_are_retried() {
    // A short lock wait gives up quickly (so queued application writes are not
    // stalled behind it) and the whole migration transaction is retried.
    // ACCESS EXCLUSIVE also blocks the catalog inspection (pg_get_indexdef),
    // so this wait comes before the script.
    let (mut f, plan) = upgrade_fixture();
    retries_through_a_brief_lock(&mut f, &plan, "ACCESS EXCLUSIVE", "pg_get_indexdef");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn brief_lock_conflicts_are_retried_at_the_lowest_statement_limit() {
    // An application's SHARE lock lets the inspection reads through, so the
    // script's ALTER TABLE is what waits. Its 1-second lock timeout must fire
    // before the statement limit, which counts from the start of the
    // statement: a statement timeout (57014) is not retried.
    let (mut f, plan) = upgrade_fixture_with_limits(Some("statement_seconds = 2"));
    let r = retries_through_a_brief_lock(&mut f, &plan, "SHARE", "ADD COLUMN label");
    assert_eq!(r["limits"]["statement_seconds"], 2, "{r}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn migration_scripts_may_run_longer_than_inspection_queries() {
    // Index builds and table rewrites take longer than catalog queries.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t(id INTEGER); SELECT pg_catalog.pg_sleep(6);",
    );
    let plan = f.plan("plan.json");
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_on_a_partitioned_table_lock_its_partitions() {
    // A check reading a partitioned parent must also block writes made
    // directly to its leaf partitions, or checked rows can change before commit.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.p(id INTEGER, at DATE NOT NULL) PARTITION BY RANGE (at);
         CREATE TABLE app.p_default PARTITION OF app.p DEFAULT;
         CREATE TABLE app.t(id INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql(
        "0002_expand.sql",
        "ALTER TABLE app.t ADD COLUMN label TEXT;",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_expand.toml"),
        "[requires]\nno_negative='SELECT NOT EXISTS (SELECT 1 FROM app.p WHERE id < 0)'\n",
    )
    .unwrap();
    let plan = f.plan("expand.json");
    let r = f.apply("expand.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    let locks = r["progress"][1]["locks"].as_array().unwrap();
    assert!(
        locks
            .iter()
            .any(|l| l["relation"] == "p_default" && l["mode"] == "ShareLock"),
        "{locks:?}"
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn policies_for_the_migration_role_match_between_scratch_and_target() {
    // TO CURRENT_USER stores the scratch role while planning and the target
    // role while executing; both are the managed role.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t(id INTEGER);
         ALTER TABLE app.t ENABLE ROW LEVEL SECURITY;
         CREATE POLICY own ON app.t TO CURRENT_USER USING (true);",
    );
    let plan = f.plan("plan.json");
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn settings_from_one_migration_do_not_reach_the_next() {
    // On the target each migration commits separately, ending SET LOCAL. One
    // replay transaction must behave the same, and session SETs must not leak.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "SET LOCAL app.mode = 'x'; SET app.session_mode = 'y'; CREATE TABLE app.a(id INTEGER);",
    );
    f.sql(
        "0002_branch.sql",
        "DO $$ BEGIN
           IF coalesce(current_setting('app.mode', true), '') = 'x'
              OR coalesce(current_setting('app.session_mode', true), '') = 'y' THEN
             CREATE TABLE app.leaked(id INTEGER);
           ELSE
             CREATE TABLE app.fresh(id INTEGER);
           END IF;
         END $$;",
    );
    let plan = f.plan("plan.json");
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert!(f
        .target()
        .query_one(
            "SELECT to_regclass('app.fresh') IS NOT NULL AND to_regclass('app.leaked') IS NULL",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn same_named_constraints_with_mixed_deferral_plan_and_migrate() {
    // Replay used to run every migration in one transaction, where SET
    // CONSTRAINTS name, which affects every constraint with that name, could
    // not restore one deferred and one immediate, so it refused this history.
    // Each migration now commits on scratch as on the target (#430), and each
    // starts with every constraint's own default.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.parent(id INTEGER PRIMARY KEY);
         CREATE TABLE app.c1(pid INTEGER, CONSTRAINT shared_fk FOREIGN KEY (pid) REFERENCES app.parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TABLE app.c2(pid INTEGER, CONSTRAINT shared_fk FOREIGN KEY (pid) REFERENCES app.parent(id) DEFERRABLE INITIALLY IMMEDIATE);",
    );
    assert_eq!(f.command("plan")["outcome"], "pass");
    f.sql(
        "0002_next.sql",
        "ALTER TABLE app.c2 ADD COLUMN note TEXT; INSERT INTO app.c1 VALUES (1); INSERT INTO app.parent VALUES (1);",
    );
    let p = f.plan("plan.json");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn partition_bounds_are_not_disclosed_in_reports() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts(email TEXT NOT NULL) PARTITION BY LIST (email);
         CREATE TABLE app.accounts_vip PARTITION OF app.accounts FOR VALUES IN ('ceo-private@example.com');",
    );
    let plan = f.plan("plan.json");
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let saved = std::fs::read_to_string(f.root.path().join("plan.json")).unwrap();
    assert!(
        !plan.to_string().contains("ceo-private"),
        "report leaks bound"
    );
    assert!(!saved.contains("ceo-private"), "saved plan leaks bound");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    let v = f.command("verify");
    assert_eq!(v["outcome"], "pass", "{v}");
    assert!(!v.to_string().contains("ceo-private"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn same_name_with_a_non_deferrable_constraint_plans_and_migrates() {
    // SET CONSTRAINTS name DEFERRED fails if any same-named constraint is not
    // deferrable, so replay in one transaction refused this group too. Each
    // migration now commits on scratch (#430), which needs no restoring.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.parent(id INTEGER PRIMARY KEY);
         CREATE TABLE app.c1(pid INTEGER, CONSTRAINT fk_user FOREIGN KEY (pid) REFERENCES app.parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TABLE app.c2(pid INTEGER, CONSTRAINT fk_user FOREIGN KEY (pid) REFERENCES app.parent(id));",
    );
    f.sql("0002_next.sql", "ALTER TABLE app.c2 ADD COLUMN note TEXT;");
    let p = f.plan("plan.json");
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn deferred_checks_get_the_script_time_limit() {
    // Deferred constraint work runs when NTNT settles the migration, after the
    // script. It must get the script's limit, not the 5-second query limit.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t(id INTEGER);
         CREATE FUNCTION app.slow_check() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_catalog.pg_sleep(6); RETURN NULL; END $$;
         CREATE CONSTRAINT TRIGGER slow AFTER INSERT ON app.t DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION app.slow_check();
         INSERT INTO app.t VALUES (1);",
    );
    let plan = f.plan("plan.json");
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
}

fn lock_strength(mode: &str) -> usize {
    [
        "AccessShareLock",
        "RowShareLock",
        "RowExclusiveLock",
        "ShareUpdateExclusiveLock",
        "ShareLock",
        "ShareRowExclusiveLock",
        "ExclusiveLock",
        "AccessExclusiveLock",
    ]
    .iter()
    .position(|m| *m == mode)
    .unwrap_or_else(|| panic!("unknown lock mode {mode}"))
}
fn strongest(locks: &Value) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::<String, String>::new();
    for l in locks.as_array().unwrap() {
        let (r, m) = (l["relation"].as_str().unwrap(), l["mode"].as_str().unwrap());
        if out
            .get(r)
            .is_none_or(|old| lock_strength(m) > lock_strength(old))
        {
            out.insert(r.to_string(), m.to_string());
        }
    }
    out
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn planned_locks_match_the_locks_migrate_takes() {
    // The plan predicts locks from SQL; migrate reports what PostgreSQL held.
    // One statement per migration, so a stronger lock from an earlier
    // statement cannot hide a wrong prediction for a later one.
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.p(id INTEGER PRIMARY KEY);
         CREATE TABLE app.t(id INTEGER, n TEXT);
         CREATE TABLE app.c(pid INTEGER);
         ALTER TABLE app.t ADD CONSTRAINT t_positive CHECK (id > 0) NOT VALID;
         CREATE FUNCTION app.noop() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .batch_execute("INSERT INTO app.p VALUES (1); INSERT INTO app.t VALUES (1, 'a'); INSERT INTO app.c VALUES (1)")
        .unwrap();
    let forms = [
        "ALTER TABLE app.t VALIDATE CONSTRAINT t_positive;",
        "CREATE INDEX t_id ON app.t(id);",
        "ALTER TABLE app.c ADD CONSTRAINT c_fk FOREIGN KEY (pid) REFERENCES app.p(id) NOT VALID;",
        "CREATE TRIGGER t_noop BEFORE INSERT ON app.t FOR EACH ROW EXECUTE FUNCTION app.noop();",
        "INSERT INTO app.p VALUES (5);",
        "ALTER TABLE app.t ADD CONSTRAINT t_short CHECK (length(n) < 100) NOT VALID;",
        "CREATE POLICY t_all ON app.t USING (true);",
    ];
    for (i, sql) in forms.iter().enumerate() {
        f.sql(&format!("{:04}_form.sql", i + 2), sql);
        let plan_file = format!("form{i}.json");
        let plan = f.plan(&plan_file);
        assert_eq!(plan["outcome"], "pass", "{sql}: {plan}");
        let predicted = strongest(&plan["plan"]["pending"][0]["analysis"]["locks"]);
        assert!(!predicted.is_empty(), "{sql}: no prediction");
        let r = f.apply(&plan_file, plan["plan_digest"].as_str());
        assert_eq!(r["outcome"], "pass", "{sql}: {r}");
        let actual = strongest(&r["progress"][i + 1]["locks"]);
        for (relation, mode) in &predicted {
            assert_eq!(
                actual.get(relation),
                Some(mode),
                "{sql}: predicted {predicted:?}, held {actual:?}"
            );
        }
    }
}

fn concurrent_fixture(index_sql: &str) -> (Fixture, Value) {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .batch_execute("INSERT INTO app.t VALUES (1), (2)")
        .unwrap();
    f.sql("0002_index.sql", index_sql);
    let plan = f.plan("upgrade.json");
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(
        plan["plan"]["pending"][0]["analysis"]["non_transactional"], true,
        "{plan}"
    );
    (f, plan)
}
fn index_state(f: &Fixture, name: &str) -> Option<bool> {
    f.target()
        .query_opt(
            "SELECT i.indisvalid FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid WHERE c.relname=$1",
            &[&name],
        )
        .unwrap()
        .map(|r| r.get(0))
}
fn ledger_rows(f: &Fixture) -> i64 {
    f.target()
        .query_one("SELECT count(*) FROM _ntnt.schema_migrations_v2", &[])
        .unwrap()
        .get(0)
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn concurrent_index_builds_while_writes_continue() {
    let (mut f, plan) = concurrent_fixture("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    // An open writer makes the build wait. A plain CREATE INDEX would queue
    // new writes behind it; a concurrent build must not.
    let mut writer = f.target();
    writer
        .batch_execute("BEGIN; INSERT INTO app.t VALUES (3)")
        .unwrap();
    let child = spawn_apply(&f, &plan);
    wait_locked(&mut f, 1);
    f.target()
        .batch_execute("SET statement_timeout='3s'; INSERT INTO app.t VALUES (4)")
        .expect("writes must continue while the index builds");
    writer.batch_execute("COMMIT").unwrap();
    let r = result(child);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "committed", "{r}");
    assert_eq!(r["progress"][1]["non_transactional"], true, "{r}");
    assert_eq!(index_state(&f, "t_id"), Some(true));
    assert_eq!(ledger_rows(&f), 2);
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn failed_concurrent_build_removes_its_invalid_index() {
    let (f, plan) = concurrent_fixture("CREATE UNIQUE INDEX CONCURRENTLY t_id ON app.t(id);");
    f.target()
        .batch_execute("INSERT INTO app.t VALUES (1)")
        .unwrap();
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_ne!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    assert_eq!(r["progress"][1]["cleanup"], "dropped_invalid_index", "{r}");
    assert_eq!(index_state(&f, "t_id"), None, "invalid index left behind");
    assert_eq!(ledger_rows(&f), 1, "the failed attempt is removed");
    assert_eq!(f.command("verify")["outcome"], "pass");
    // Fix the data and rerun the same reviewed plan.
    f.target()
        .batch_execute("DELETE FROM app.t WHERE ctid = (SELECT max(ctid) FROM app.t WHERE id = 1)")
        .unwrap();
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(index_state(&f, "t_id"), Some(true));
}

fn reconcile(f: &Fixture, plan: &Value) -> Value {
    let output = f
        .process("reconcile")
        .args(["--plan", "upgrade.json"])
        .env("DB_APPROVAL", plan["plan_digest"].as_str().unwrap())
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}
/// Block until the target database has no session other than this one, so
/// work a disconnected client started has finished. The whole wait runs on a
/// helper thread and is bounded from outside: a PostgreSQL call that never
/// returns fails the test after 30 s instead of hanging it. Each query is
/// also bounded in the server by `statement_timeout`.
fn wait_for_other_target_sessions_to_end(f: &Fixture) {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
    // connect_timeout bounds the connection attempt; statement_timeout below
    // bounds each query in the server.
    let url = format!("{} connect_timeout=5", f.target_url);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut client =
                postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
            client
                .batch_execute("SET statement_timeout = '5s'")
                .map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + LIMIT;
            loop {
                let others: Vec<String> = client
                    .query(
                        "SELECT format('%s %s %s', pid, state, left(query, 60)) FROM pg_stat_activity
                         WHERE datname = current_database() AND pid <> pg_backend_pid()
                           AND backend_type = 'client backend'",
                        &[],
                    )
                    .map_err(|e| e.to_string())?
                    .iter()
                    .map(|r| r.get(0))
                    .collect();
                if others.is_empty() {
                    return Ok(());
                }
                if std::time::Instant::now() >= deadline {
                    return Err(format!(
                        "target sessions still open after {LIMIT:?}: {others:?}"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        })();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(LIMIT + std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("{error}"),
        Err(_) => panic!("waiting for target sessions did not return within {LIMIT:?}"),
    }
}
fn attempt_state(f: &Fixture) -> Option<String> {
    f.target()
        .query_opt(
            "SELECT state FROM _ntnt.schema_migrations_v2 WHERE position=2",
            &[],
        )
        .unwrap()
        .map(|r| r.get(0))
}
/// Kill `migrate` while its build waits on an open writer. The orphaned
/// backend keeps the session (and the migrator lock) until it ends.
fn interrupted_build(index_sql: &str) -> (Fixture, Value, postgres::Client) {
    let (mut f, plan) = concurrent_fixture(index_sql);
    let mut writer = f.target();
    writer
        .batch_execute("BEGIN; INSERT INTO app.t VALUES (3)")
        .unwrap();
    let mut child = spawn_apply(&f, &plan);
    wait_locked(&mut f, 1);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(attempt_state(&f).as_deref(), Some("running"));
    (f, plan, writer)
}
fn build_backends(f: &mut Fixture) -> i64 {
    f.admin
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE usename=$1 AND query ILIKE 'CREATE%INDEX%'",
            &[&f.target],
        )
        .unwrap()
        .get(0)
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn interrupted_build_blocks_later_work_until_reconciled() {
    let (mut f, plan, writer) = interrupted_build("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    // The build's session still owns the migrators' lock: no second run starts.
    let r = reconcile(&f, &plan);
    assert_ne!(r["outcome"], "pass", "{r}");
    f.admin
        .batch_execute(&format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename='{}' AND query ILIKE 'CREATE%INDEX%'", f.target))
        .unwrap();
    drop(writer);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while build_backends(&mut f) > 0 {
        assert!(
            std::time::Instant::now() < until,
            "build backend did not end"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(index_state(&f, "t_id"), Some(false));

    // Inspection, planning and the same plan all refuse to continue.
    assert!(code(&f.command("status"), "DB_ATTEMPT_UNRESOLVED"));
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_ATTEMPT_UNRESOLVED"), "{r}");
    assert_eq!(
        index_state(&f, "t_id"),
        Some(false),
        "migrate must not drop it"
    );
    f.sql("0003_more.sql", "CREATE TABLE app.u(id INTEGER);");
    assert!(code(&f.command("plan"), "DB_ATTEMPT_UNRESOLVED"));
    std::fs::remove_file(f.root.path().join("migrations/0003_more.sql")).unwrap();

    // Reconciliation needs the same approval, then rolls the attempt back.
    let unapproved = f
        .process("reconcile")
        .args(["--plan", "upgrade.json"])
        .output()
        .unwrap();
    let unapproved: Value = serde_json::from_slice(&unapproved.stdout).unwrap();
    assert!(code(&unapproved, "DB_APPROVAL_REQUIRED"), "{unapproved}");
    let r = reconcile(&f, &plan);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["reconciliation"]["resolution"], "rolled_back", "{r}");
    assert_eq!(r["reconciliation"]["observed_index"], "invalid", "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(attempt_state(&f), None);
    assert_eq!(f.command("verify")["outcome"], "pass");
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(index_state(&f, "t_id"), Some(true));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn interrupted_build_that_finished_reconciles_as_completed() {
    let (mut f, plan, writer) = interrupted_build("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    // Let the orphaned build finish on its own after migrate died.
    writer.close().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while build_backends(&mut f) > 0 || index_state(&f, "t_id") != Some(true) {
        assert!(
            std::time::Instant::now() < until,
            "orphaned build did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(
        code(&r, "DB_ATTEMPT_UNRESOLVED"),
        "a valid index is not recorded silently: {r}"
    );
    let r = reconcile(&f, &plan);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["reconciliation"]["resolution"], "completed", "{r}");
    assert_eq!(attempt_state(&f).as_deref(), Some("applied"));
    assert_eq!(f.command("verify")["outcome"], "pass");
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "already_applied", "{r}");
}

/// `plan_digest` as `ntnt db` computes it: SHA-256 over the length-framed
/// compact JSON of the plan with sorted keys.
fn plan_digest(plan: &Value) -> String {
    use sha2::{Digest, Sha256};
    fn sort(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.sort_keys();
                map.values_mut().for_each(sort);
            }
            Value::Array(values) => values.iter_mut().for_each(sort),
            _ => {}
        }
    }
    let mut plan = plan.clone();
    sort(&mut plan);
    let bytes = serde_json::to_vec(&plan).unwrap();
    let mut hash = Sha256::new();
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(&bytes);
    hex::encode(hash.finalize())
}
/// Rewrite the saved `upgrade.json` as ntnt 0.6.1 saved plans: 0.6.1 connected
/// only with sslmode=disable and bound no `transport` into the target.
fn saved_by_0_6_1(f: &Fixture, path: &str) -> String {
    let mut saved: Value =
        serde_json::from_slice(&std::fs::read(f.root.path().join("upgrade.json")).unwrap())
            .unwrap();
    assert_eq!(plan_digest(&saved["plan"]), saved["plan_digest"]);
    let target = saved["plan"]["binding"]["target"].as_object_mut().unwrap();
    assert_eq!(target.remove("transport").unwrap()["sslmode"], "disable");
    let digest = plan_digest(&saved["plan"]);
    saved["plan_digest"] = Value::String(digest.clone());
    std::fs::write(
        f.root.path().join(path),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    digest
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn attempt_interrupted_under_0_6_1_reconciles_with_its_plan() {
    /// Only a hang reaches this: a terminated backend ends promptly.
    const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(60);
    let (mut f, _, writer) = interrupted_build("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    f.admin
        .batch_execute(&format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename='{}' AND query ILIKE 'CREATE%INDEX%'", f.target))
        .unwrap();
    drop(writer);
    // Wait for the terminated build's session to disappear.
    let guard = std::time::Instant::now() + HANG_GUARD;
    while build_backends(&mut f) > 0 {
        assert!(
            std::time::Instant::now() < guard,
            "build backend did not end within {HANG_GUARD:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(index_state(&f, "t_id"), Some(false));
    // After upgrading, no new plan can be made until the attempt is resolved,
    // so reconcile must accept the plan 0.6.1 saved.
    assert!(code(&f.command("plan"), "DB_ATTEMPT_UNRESOLVED"));
    let digest = saved_by_0_6_1(&f, "legacy.json");
    let with_legacy_plan = |command: &str| {
        let output = f
            .process(command)
            .args(["--plan", "legacy.json"])
            .env("DB_APPROVAL", &digest)
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    };
    let r = with_legacy_plan("reconcile");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["reconciliation"]["resolution"], "rolled_back", "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(attempt_state(&f), None);
    // migrate still needs a plan made by this release.
    let r = with_legacy_plan("migrate");
    assert!(code(&r, "DB_PLAN_STALE"), "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
    let fresh = f.plan("fresh.json");
    let r = f.apply("fresh.json", fresh["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(index_state(&f, "t_id"), Some(true));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn reconciliation_refuses_what_it_cannot_explain() {
    // A crash after recording the attempt but before the build started.
    let (f, plan) = concurrent_fixture("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    let checksum = plan["plan"]["pending"][0]["checksum"].as_str().unwrap();
    f.target()
        .execute(
            "INSERT INTO _ntnt.schema_migrations_v2 VALUES(2,'2','0002_index.sql',$1,'running','app','app')",
            &[&checksum],
        )
        .unwrap();
    // An index of the planned name with another definition is not explained
    // by the attempt: nothing is changed.
    f.target()
        .batch_execute("CREATE INDEX t_id ON app.t((id + 1))")
        .unwrap();
    let r = reconcile(&f, &plan);
    assert!(code(&r, "DB_RECONCILE_MISMATCH"), "{r}");
    assert_eq!(attempt_state(&f).as_deref(), Some("running"));
    assert_eq!(index_state(&f, "t_id"), Some(true));
    f.target().batch_execute("DROP INDEX app.t_id").unwrap();
    let r = reconcile(&f, &plan);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["reconciliation"]["resolution"], "not_started", "{r}");
    assert_eq!(attempt_state(&f), None);
    // Nothing left to reconcile.
    let r = reconcile(&f, &plan);
    assert_eq!(r["reconciliation"]["resolution"], "none", "{r}");
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn lost_reconcile_commit_is_unknown_not_failed() {
    use db_proxy::{Fault, Proxy};
    let (mut f, plan) = concurrent_fixture("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    let checksum = plan["plan"]["pending"][0]["checksum"].as_str().unwrap();
    f.target()
        .execute(
            "INSERT INTO _ntnt.schema_migrations_v2 VALUES(2,'2','0002_index.sql',$1,'running','app','app')",
            &[&checksum],
        )
        .unwrap();
    let proxy = Proxy::new(Fault::AfterCommitUnavailable, 1);
    let direct = f.target_url.clone();
    f.target_url = direct.replace(&std::env::var("NTNT_DB_TEST_HOST").unwrap(), proxy.host());
    let r = reconcile(&f, &plan);
    assert!(proxy.fired.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(r["outcome"], "unknown", "{r}");
    assert_eq!(r["reconciliation"]["pending_write"], "not_started", "{r}");
    f.target_url = direct;
    // The proxy dropped only the acknowledgement, so the backend that ran the
    // COMMIT may still be finishing it. Wait for every other session on the
    // target to end (bounded) before checking that the commit landed.
    wait_for_other_target_sessions_to_end(&f);
    // The commit did land; a fresh reconcile sees nothing left to do.
    assert_eq!(attempt_state(&f), None);
    assert_eq!(reconcile(&f, &plan)["reconciliation"]["resolution"], "none");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn unrecorded_index_of_the_planned_name_is_drift_not_progress() {
    // Without a running attempt, an index nobody recorded is not evidence
    // that this migration ran.
    let (f, plan) = concurrent_fixture("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    f.target()
        .batch_execute("CREATE INDEX CONCURRENTLY t_id ON app.t(id)")
        .unwrap();
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_PLAN_STALE"), "{r}");
    assert_eq!(ledger_rows(&f), 1);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn concurrent_index_migrations_cannot_have_checks() {
    // Checks need a transaction around the change; there is none here.
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    f.sql(
        "0002_index.sql",
        "CREATE INDEX CONCURRENTLY t_id ON app.t(id);",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_index.toml"),
        "[ensures]\nok='SELECT true'\n",
    )
    .unwrap();
    let r = f.command("plan");
    assert!(code(&r, "DB_REPLAY_UNSUPPORTED"), "{r}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn recovery_never_drops_an_invalid_index_it_did_not_plan() {
    // Same name, different definition: not ours to drop.
    let (f, plan) = concurrent_fixture("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    f.target()
        .batch_execute("INSERT INTO app.t VALUES (1)")
        .unwrap();
    assert!(f
        .target()
        .batch_execute("CREATE UNIQUE INDEX CONCURRENTLY t_id ON app.t((id * 0))")
        .is_err());
    assert_eq!(index_state(&f, "t_id"), Some(false));
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_ne!(r["outcome"], "pass", "{r}");
    assert_eq!(
        index_state(&f, "t_id"),
        Some(false),
        "unplanned index was dropped"
    );
    assert_eq!(ledger_rows(&f), 1);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn concurrent_builds_refuse_custom_casts() {
    // Index expressions coerce values; a custom implicit cast on the target
    // could run code that replay never ran.
    let (f, plan) = concurrent_fixture("CREATE INDEX CONCURRENTLY t_id ON app.t(id);");
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    config.dbname(&f.target);
    let mut admin = config.connect(postgres::NoTls).unwrap();
    admin
        .batch_execute("CREATE FUNCTION public.escape(integer) RETURNS text LANGUAGE sql AS $$ SELECT 'x' $$; CREATE CAST (integer AS text) WITH FUNCTION public.escape(integer) AS ASSIGNMENT")
        .unwrap();
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_CHECK_UNSUPPORTED"), "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
}

/// Replace the manifest's `[databases.app.limits]` table, which stays last.
fn set_limits(f: &Fixture, limits: &str) {
    let path = f.root.path().join("ntnt.toml");
    let manifest = std::fs::read_to_string(&path).unwrap();
    let base = manifest.split("[databases.app.limits]").next().unwrap();
    std::fs::write(&path, format!("{base}[databases.app.limits]\n{limits}\n")).unwrap();
}
/// PostgreSQL cancelled a statement at its time limit (SQLSTATE 57014).
fn statement_timed_out(r: &Value) -> bool {
    r["diagnostics"].as_array().unwrap().iter().any(|d| {
        d["code"] == "DB_POSTGRES" && d["message"].as_str().is_some_and(|m| m.contains("57014"))
    })
}

/// The script records the statement limit it runs under, then waits for
/// advisory lock 346 with no lock timeout, so a test holding that lock on
/// the target decides how long it runs. Replay on the scratch database does
/// not wait: advisory locks are per database.
const WAITS_FOR_THE_TEST: &str = "CREATE TABLE app.t(id INTEGER, statement_limit TEXT); INSERT INTO app.t VALUES (1, pg_catalog.current_setting('statement_timeout')); SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(346);";
/// The statement limits recorded in `app.t`, in seconds, by id.
fn recorded_limits(f: &Fixture) -> Vec<i64> {
    f.target()
        .query(
            "SELECT extract(epoch FROM statement_limit::interval)::bigint FROM app.t ORDER BY id",
            &[],
        )
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn statement_limit_bounds_scripts_and_a_raised_limit_needs_a_new_plan() {
    // Scaled stand-in for a migration that outgrows the default limit on a
    // large table: 2 seconds (the lowest accepted) plays the default, and a
    // raised limit lets the same script run past it.
    let mut f = Fixture::new();
    f.sql("0001_slow.sql", WAITS_FOR_THE_TEST);
    let mut holder = f.target();
    holder
        .batch_execute("SELECT pg_advisory_lock(346)")
        .unwrap();
    set_limits(&f, "statement_seconds = 2");
    // Replay on the empty scratch database keeps the default limit.
    let low = f.plan("low.json");
    assert_eq!(
        low["plan"]["execution"]["limits"],
        serde_json::json!({"statement_seconds": 2, "migrator_wait_seconds": 5}),
        "{low}"
    );
    // The lock is held until after the outcome: only the limit ends the wait.
    let r = result_within(spawn_plan(&f, "low.json", &low));
    assert!(statement_timed_out(&r), "{r}");
    assert_eq!(r["progress"][0]["state"], "failed", "{r}");
    assert_eq!(r["cleanup"], "pass", "{r}");
    assert_eq!(r["limits"]["statement_seconds"], 2, "{r}");
    // The whole migration transaction rolled back, ledger bootstrap included.
    assert!(f
        .target()
        .query_one(
            "SELECT to_regclass('app.t') IS NULL AND to_regnamespace('_ntnt') IS NULL",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    // A changed limit is a changed policy: the reviewed plan is stale.
    set_limits(&f, &raised("statement_seconds"));
    let stale = f.apply("low.json", low["plan_digest"].as_str());
    assert!(code(&stale, "DB_PLAN_STALE"), "{stale}");
    let against = f.command_args("verify", &["--against", "low.json"]);
    assert_eq!(against["plan_validation"]["outcome"], "fail", "{against}");
    let raised_plan = f.plan("raised.json");
    let child = spawn_plan(&f, "raised.json", &raised_plan);
    let (_, _, query) = lock_waiter(&mut f);
    assert!(query.contains("pg_advisory_xact_lock(346)"), "{query}");
    // Keep the script running past the low limit and the 5-second defaults.
    std::thread::sleep(PAST_THE_DEFAULTS);
    holder
        .batch_execute("SELECT pg_advisory_unlock_all()")
        .unwrap();
    let r = result_within(child);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
    let raised_seconds = 10 * PAST_THE_DEFAULTS.as_secs();
    assert_eq!(
        r["limits"],
        serde_json::json!({"statement_seconds": raised_seconds,
            "migrator_wait_seconds": 5, "work_deadline_seconds": 100,
            "total_deadline_seconds": 120}),
        "{r}"
    );
    // Each script runs under exactly the limit its plan binds: the raised
    // one above, and the low one for a script planned with it.
    f.sql(
        "0002_record.sql",
        "INSERT INTO app.t VALUES (2, pg_catalog.current_setting('statement_timeout'));",
    );
    set_limits(&f, "statement_seconds = 2");
    let record = f.plan("record.json");
    let r = f.apply("record.json", record["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(recorded_limits(&f), [raised_seconds as i64, 2]);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn statement_limit_bounds_a_concurrent_index_build() {
    // A build that waits on a busy table, scaled: at a 2-second limit it
    // fails and cleans up as before; a raised limit lets it wait past that.
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql(
        "0002_index.sql",
        "CREATE INDEX CONCURRENTLY t_id ON app.t(id);",
    );
    set_limits(&f, "statement_seconds = 2");
    let low = f.plan("low.json");
    // SHARE conflicts with the build's SHARE UPDATE EXCLUSIVE, so the build
    // waits before it creates anything. The lock is held until after the
    // outcome, so only the limit ends the wait; the 90-second default would
    // run into HANG_GUARD.
    let mut holder = f.target();
    holder
        .batch_execute("BEGIN; LOCK TABLE app.t IN SHARE MODE")
        .unwrap();
    let r = result_within(spawn_plan(&f, "low.json", &low));
    assert!(statement_timed_out(&r), "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    assert_eq!(r["progress"][1]["cleanup"], "pass", "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(ledger_rows(&f), 1, "the failed attempt is removed");
    set_limits(&f, &raised("statement_seconds"));
    let raised_plan = f.plan("raised.json");
    let child = spawn_plan(&f, "raised.json", &raised_plan);
    let (_, _, query) = lock_waiter(&mut f);
    assert!(query.contains("CREATE INDEX CONCURRENTLY"), "{query}");
    // Keep the build waiting past the low limit and the 5-second defaults.
    std::thread::sleep(PAST_THE_DEFAULTS);
    holder.batch_execute("COMMIT").unwrap();
    let r = result_within(child);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "committed", "{r}");
    assert_eq!(index_state(&f, "t_id"), Some(true));
    assert_eq!(f.command("verify")["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn migrator_wait_bounds_the_wait_for_a_competing_migrator() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let mut other = f.target();
    other
        .batch_execute("SELECT pg_advisory_lock(1853124212,245)")
        .unwrap();
    // Shorter than the 5-second default. The competitor keeps the lock until
    // after the outcome, so the wait runs out, as a lock conflict: not the
    // statement timeout that used to cut it off (57014) or the client
    // deadline.
    set_limits(&f, "migrator_wait_seconds = 1");
    let short = f.plan("short.json");
    let r = result_within(spawn_plan(&f, "short.json", &short));
    assert!(code(&r, "DB_LOCK_CONFLICT"), "{r}");
    assert!(!statement_timed_out(&r) && !code(&r, "DB_TIMEOUT"), "{r}");
    assert_eq!(r["progress"][0]["state"], "not_run", "{r}");
    assert_eq!(r["limits"]["migrator_wait_seconds"], 1, "{r}");
    // Longer than the old 5-second cap on this wait (its lock, statement and
    // client limits): outwait a competitor that keeps the lock for
    // PAST_THE_DEFAULTS after the migrator is seen waiting for it.
    set_limits(&f, &raised("migrator_wait_seconds"));
    let patient = f.plan("patient.json");
    let child = spawn_plan(&f, "patient.json", &patient);
    let (_, _, query) = lock_waiter(&mut f);
    assert!(
        query.contains("pg_advisory_lock(1853124212, 245)"),
        "{query}"
    );
    std::thread::sleep(PAST_THE_DEFAULTS);
    other
        .batch_execute("SELECT pg_advisory_unlock_all()")
        .unwrap();
    let r = result_within(child);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
}

/// Migrate a fresh history, asserting each step passes.
fn migrate_all(f: &Fixture, path: &str) -> Value {
    let plan = f.plan(path);
    let r = f.apply(path, plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    r
}
fn drift_names(r: &Value, object: &str, change: &str) -> bool {
    r["drift"]["differences"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["object"] == object && d["change"] == change)
}

/// CodeRabbit on #445 (adoption.rs:34): a managed schema named `user` needs
/// quotes in SQL. An adoption with no adoption_checks is refused with an
/// example check; added to the manifest as the advice gives it, the check is
/// accepted: the adoption plans, and migrates with the check run and passed.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn the_example_adoption_check_quotes_a_schema_named_user() {
    let f = Fixture::with_schema("user");
    let accounts = "CREATE TABLE \"user\".accounts (id int PRIMARY KEY, email text NOT NULL);";
    f.sql("0001_accounts.sql", accounts);
    f.target()
        .batch_execute(&format!(
            "{accounts} INSERT INTO \"user\".accounts VALUES (1, 'a@example.com');"
        ))
        .unwrap();
    let refused = f.command_args("plan", &["--adopt-through", "1"]);
    assert!(code(&refused, "DB_ADOPTION_INPUT"), "{refused}");
    let advice = refused["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_ADOPTION_INPUT")
        .unwrap()["next_action"]
        .as_str()
        .unwrap()
        .to_owned();
    let example = advice
        .split_once("such as ")
        .and_then(|(_, rest)| rest.split_once(", then plan again"))
        .map(|(example, _)| example)
        .unwrap_or_else(|| panic!("no example check in {advice:?}"));
    let manifest = f.root.path().join("ntnt.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(&format!("\n[databases.app.adoption_checks]\n{example}\n"));
    std::fs::write(&manifest, text).unwrap();
    let plan = f.command_args("plan", &["--adopt-through", "1", "--out", "adoption.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let adopted = f.apply("adoption.json", plan["plan_digest"].as_str());
    assert_eq!(adopted["outcome"], "pass", "{example}: {adopted}");
    assert_eq!(adopted["assertions"]["executed"], 1, "{adopted}");
    assert_eq!(
        example,
        r#"no_null_email = "SELECT NOT EXISTS (SELECT 1 FROM \"user\".accounts WHERE email IS NULL)""#
    );
}

/// #420: a schema the history creates is managed through its whole life: a
/// foreign key from it into the managed schema is not an external dependency,
/// a later rename or DROP SCHEMA replays like any other change, and drift is
/// reported under its current name.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn created_schemas_are_managed_across_foreign_keys_renames_and_drops() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id int PRIMARY KEY);
CREATE SCHEMA audit;
CREATE TABLE audit.log (id int PRIMARY KEY, account int REFERENCES app.accounts(id));",
    );
    migrate_all(&f, "first.json");
    // The next migration runs while audit.log references app.accounts.
    f.sql(
        "0002_email.sql",
        "ALTER TABLE app.accounts ADD COLUMN email text;",
    );
    migrate_all(&f, "second.json");
    f.sql(
        "0003_archive.sql",
        "ALTER SCHEMA audit RENAME TO audit_archive;",
    );
    migrate_all(&f, "third.json");
    let renamed = f.command("verify");
    assert_eq!(renamed["outcome"], "pass", "{renamed}");
    let objects = renamed["catalog"]["objects"].as_object().unwrap();
    assert!(
        objects.contains_key("audit_archive.relation:log"),
        "{:?}",
        objects.keys()
    );
    assert!(
        !objects
            .keys()
            .any(|k| k.starts_with("audit.") || k == "schema:audit"),
        "{:?}",
        objects.keys()
    );
    // A privilege change out of band is drift under the schema's new name.
    f.target()
        .batch_execute("GRANT SELECT ON audit_archive.log TO PUBLIC")
        .unwrap();
    let drift = f.command("verify");
    assert_eq!(drift["outcome"], "fail", "{drift}");
    assert!(
        drift_names(&drift, "audit_archive.relation:log", "changed"),
        "{}",
        drift["drift"]
    );
    f.target()
        .batch_execute("REVOKE SELECT ON audit_archive.log FROM PUBLIC")
        .unwrap();
    f.sql(
        "0004_retire.sql",
        "DROP TABLE audit_archive.log; DROP SCHEMA audit_archive;",
    );
    migrate_all(&f, "fourth.json");
    assert_eq!(f.command("verify")["outcome"], "pass");
    // Recreating the dropped schema out of band is drift too.
    f.target()
        .batch_execute("CREATE SCHEMA audit_archive")
        .unwrap();
    let recreated = f.command("verify");
    assert_eq!(recreated["outcome"], "fail", "{recreated}");
    assert!(
        drift_names(&recreated, "schema:audit_archive", "unexpected"),
        "{}",
        recreated["drift"]
    );
}

/// Adopting an existing database compares the schemas the history creates.
/// A managed view over one of them is covered, so adoption checks may run.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn adoption_compares_the_schemas_the_history_creates() {
    use std::io::Write;
    let f = Fixture::new();
    let schema = "CREATE TABLE app.accounts (id int PRIMARY KEY);
CREATE SCHEMA audit;
CREATE TABLE audit.log (id int PRIMARY KEY, msg text);
CREATE VIEW app.recent AS SELECT id, msg FROM audit.log;";
    f.sql("0001_initial.sql", schema);
    f.target().batch_execute(schema).unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(f.root.path().join("ntnt.toml"))
            .unwrap(),
        "\n[databases.app.adoption_checks]\naccounts='SELECT count(*)=0 FROM app.accounts'"
    )
    .unwrap();
    f.target()
        .batch_execute("ALTER TABLE audit.log ADD COLUMN unplanned text")
        .unwrap();
    let blocked = f.command_args("plan", &["--adopt-through", "1"]);
    assert_eq!(blocked["outcome"], "fail", "{blocked}");
    assert!(
        drift_names(&blocked, "audit.column:log.unplanned", "unexpected"),
        "{}",
        blocked["drift"]
    );
    f.target()
        .batch_execute("ALTER TABLE audit.log DROP COLUMN unplanned")
        .unwrap();
    let plan = f.command_args("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let r = f.apply("adopt.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

/// `reconcile` compares the schemas the history creates, like migrate.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn reconcile_compares_the_schemas_the_history_creates() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t (id int); CREATE SCHEMA audit; CREATE TABLE audit.log (id int);",
    );
    migrate_all(&f, "initial.json");
    f.sql(
        "0002_index.sql",
        "CREATE INDEX CONCURRENTLY t_id ON app.t(id);",
    );
    let plan = f.plan("upgrade.json");
    assert_eq!(
        plan["plan"]["pending"][0]["analysis"]["non_transactional"], true,
        "{plan}"
    );
    // The state migrate leaves when it stops after recording the attempt and
    // before the build starts.
    f.target()
        .execute(
            "INSERT INTO _ntnt.schema_migrations_v2 VALUES (2, '2', '0002_index.sql', $1, 'running', 'app', 'app')",
            &[&plan["source"]["migrations"][1]["checksum"].as_str().unwrap()],
        )
        .unwrap();
    f.target()
        .batch_execute("ALTER TABLE audit.log ADD COLUMN unplanned int")
        .unwrap();
    let mismatch = reconcile(&f, &plan);
    assert!(code(&mismatch, "DB_RECONCILE_MISMATCH"), "{mismatch}");
    assert_eq!(attempt_state(&f).as_deref(), Some("running"));
    f.target()
        .batch_execute("ALTER TABLE audit.log DROP COLUMN unplanned")
        .unwrap();
    let r = reconcile(&f, &plan);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["reconciliation"]["resolution"], "not_started", "{r}");
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}

/// Josh's decision on #420 (2026-11-21): statements outside the managed
/// scope run on the target as they did before #420. REVOKE and GRANT on
/// public, a GRANT on a table the history creates there and a DROP of
/// another application's table plan, migrate and verify.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn statements_outside_the_managed_scope_migrate_and_verify() {
    let f = Fixture::new();
    // Another application's table in public.
    f.target()
        .batch_execute("CREATE TABLE public.legacy (id int)")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id int PRIMARY KEY);
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO PUBLIC;
CREATE TABLE public.audit_log (id int PRIMARY KEY, account int);
GRANT SELECT ON public.audit_log TO PUBLIC;
DROP TABLE IF EXISTS public.legacy;",
    );
    migrate_all(&f, "first.json");
    // Each statement ran on the target as written.
    assert!(f
        .target()
        .query_one(
            "SELECT to_regclass('public.legacy') IS NULL AND has_table_privilege('public', 'public.audit_log', 'SELECT')",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    let verified = f.command("verify");
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["diagnostics"], serde_json::json!([]), "{verified}");
    // A later migration drops the table it created there.
    f.sql("0002_retire.sql", "DROP TABLE public.audit_log;");
    migrate_all(&f, "second.json");
    let verified = f.command("verify");
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["diagnostics"], serde_json::json!([]), "{verified}");
}

/// A history applied before #420 that revokes CREATE on public keeps
/// verifying with no diagnostic. #420 refused the statement when replay
/// reached it, so verify could no longer replay the applied prefix.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn an_applied_history_that_revokes_create_on_public_keeps_verifying() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t (id int);\nREVOKE CREATE ON SCHEMA public FROM PUBLIC;");
    let verified = f.command("verify");
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["history"]["applied_count"], 1, "{verified}");
    assert_eq!(verified["diagnostics"], serde_json::json!([]), "{verified}");
}

/// A pending statement that may act outside the managed scope is flagged in
/// the plan the way a destructive change is: a hazard on the migration, and
/// an `outside_scope` entry naming the statement and the schema for each
/// qualified name or IN SCHEMA list, each search_path that names another
/// schema, and each schema-qualified name in SQL NTNT cannot analyze. Nothing
/// is refused: the plan stays reviewable, is approved and migrates, and once
/// applied the migration is not flagged again.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn pending_statements_outside_the_managed_scope_are_flagged_for_review() {
    let f = Fixture::new();
    f.target()
        .batch_execute("CREATE TABLE public.legacy (id int)")
        .unwrap();
    f.sql("0001_initial.sql", "CREATE TABLE app.t (id int);");
    migrate_all(&f, "first.json");
    f.sql(
        "0002_outside.sql",
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC;
SET LOCAL search_path TO public;
DROP TABLE IF EXISTS legacy;
SELECT pg_catalog.set_config('search_path', 'app', true);
ALTER SEQUENCE IF EXISTS public.s RESTART WITH 5;
GRANT EXECUTE ON ALL ROUTINES IN SCHEMA public TO PUBLIC;
ALTER TABLE app.t ADD COLUMN note text;",
    );
    let plan = f.plan("second.json");
    assert_eq!(plan["plan"]["classification"], "requires_review", "{plan}");
    let pending = plan["plan"]["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0]["source"], "0002_outside.sql");
    let analysis = &pending[0]["analysis"];
    assert!(
        names_in(&analysis["hazards"]).contains(&"OUTSIDE_MANAGED_SCOPE"),
        "{analysis}"
    );
    assert_eq!(
        analysis["outside_scope"],
        serde_json::json!([
            {"statement": 1, "schema": "public", "via": "name"},
            {"statement": 2, "schema": "public", "via": "search_path"},
            {"statement": 5, "schema": "public", "via": "token"},
            {"statement": 6, "schema": "public", "via": "token"}
        ]),
        "{analysis}"
    );
    // The human plan names each one under its migration.
    let human = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "plan", "--target", "test"])
        .env("DB_TARGET", &f.target_url)
        .env("DB_SCRATCH", &f.scratch_url)
        .env_remove("DB_APPROVAL")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(f.root.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&human.stdout);
    for line in [
        "Pending \"0002_outside.sql\"",
        "  Outside the managed scope: statement 1 names schema \"public\"",
        "  Outside the managed scope: statement 2 puts schema \"public\" on the search path",
        "  Outside the managed scope: statement 5 names schema \"public\" in SQL NTNT does not analyze",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    let applied = f.apply("second.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    // The statements ran as written: statement 3 dropped public.legacy.
    assert!(f
        .target()
        .query_one("SELECT to_regclass('public.legacy') IS NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
    let verified = f.command("verify");
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["diagnostics"], serde_json::json!([]), "{verified}");
    let again = f.command("plan");
    assert_eq!(again["plan"]["pending"], serde_json::json!([]), "{again}");
}

fn names_in(list: &Value) -> Vec<&str> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Item 2 of Josh's decision on #420: an object the history creates in a
/// schema it did not create (public here) is inspected, and only that. Its
/// columns, constraints, indexes and privileges follow it, a managed view
/// over it is covered, and its foreign key into the managed schema is no
/// external dependency. Other objects in public, such as another
/// application's tables, and public itself are never reported.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn objects_the_history_creates_outside_the_scope_are_inspected() {
    let f = Fixture::new();
    // Another application's table in public, there before the first plan.
    f.target()
        .batch_execute("CREATE TABLE public.other_app (id int)")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id int PRIMARY KEY);
CREATE TABLE public.audit_log (id int PRIMARY KEY, account int REFERENCES app.accounts (id), msg text);
CREATE INDEX audit_log_msg ON public.audit_log (msg);
CREATE VIEW app.recent AS SELECT id, msg FROM public.audit_log;",
    );
    let plan = f.plan("first.json");
    assert_eq!(
        plan["plan"]["execution"]["outside_objects"],
        serde_json::json!(["public.relation:audit_log"]),
        "{plan}"
    );
    let applied = f.apply("first.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let clean = f.command("verify");
    assert_eq!(clean["outcome"], "pass", "{clean}");
    assert_eq!(clean["diagnostics"], serde_json::json!([]), "{clean}");
    assert_eq!(
        clean["target"]["outside_objects"],
        serde_json::json!(["public.relation:audit_log"])
    );
    let objects = clean["catalog"]["objects"].as_object().unwrap();
    for key in [
        "public.relation:audit_log",
        "public.column:audit_log.msg",
        "public.constraint:audit_log.audit_log_account_fkey",
        "public.index:audit_log_pkey",
        "public.index:audit_log_msg",
        "view:recent",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    assert!(
        !objects
            .keys()
            .any(|k| k.contains("other_app") || k == "schema:public"),
        "{:?}",
        objects.keys()
    );
    // The next migration runs while public.audit_log references app.accounts.
    f.sql(
        "0002_email.sql",
        "ALTER TABLE app.accounts ADD COLUMN email text;",
    );
    migrate_all(&f, "second.json");
    // Objects in public the history did not create, and public's own
    // privileges, are never reported.
    f.target()
        .batch_execute(
            "CREATE TABLE public.unrelated (id int); GRANT CREATE ON SCHEMA public TO PUBLIC",
        )
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["outcome"], "pass", "{r}");
    for (change, object, kind, undo) in [
        (
            "ALTER TABLE public.audit_log ADD COLUMN extra int",
            "public.column:audit_log.extra",
            "unexpected",
            "ALTER TABLE public.audit_log DROP COLUMN extra",
        ),
        (
            "CREATE INDEX audit_log_extra ON public.audit_log (id, msg)",
            "public.index:audit_log_extra",
            "unexpected",
            "DROP INDEX public.audit_log_extra",
        ),
        (
            "GRANT SELECT ON public.audit_log TO PUBLIC",
            "public.relation:audit_log",
            "changed",
            "REVOKE SELECT ON public.audit_log FROM PUBLIC",
        ),
        (
            "DROP VIEW app.recent; DROP TABLE public.audit_log",
            "public.relation:audit_log",
            "missing",
            "",
        ),
    ] {
        f.target().batch_execute(change).unwrap();
        let r = f.command("verify");
        assert_eq!(r["outcome"], "fail", "{change}: {r}");
        assert!(code(&r, "DB_DRIFT"), "{change}: {r}");
        assert!(drift_names(&r, object, kind), "{change}: {}", r["drift"]);
        if !undo.is_empty() {
            f.target().batch_execute(undo).unwrap();
            let r = f.command("verify");
            assert_eq!(r["outcome"], "pass", "{undo}: {r}");
        }
    }
}

/// CodeRabbit 4656058134 on #445: a table the history does not create, with
/// a foreign key into a table the history creates outside the managed scope,
/// blocks execution as one into the managed scope does, with the same
/// refusal, so an approved DELETE cannot cascade into its rows. Foreign keys
/// between tables the history creates, managed and outside, in either
/// direction, still migrate, and the DELETE's cascade reaches them.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn foreign_keys_into_tables_the_history_creates_outside_block_execution() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id int PRIMARY KEY);
CREATE TABLE public.audit_log (id int PRIMARY KEY, account int REFERENCES app.accounts (id));
CREATE TABLE public.audit_detail (id int PRIMARY KEY, log int REFERENCES public.audit_log (id) ON DELETE CASCADE);
CREATE TABLE app.entries (id int PRIMARY KEY, log int REFERENCES public.audit_log (id) ON DELETE CASCADE);",
    );
    migrate_all(&f, "first.json");
    f.sql(
        "0002_seed.sql",
        "INSERT INTO app.accounts VALUES (1);
INSERT INTO public.audit_log VALUES (1, 1), (2, 1);
INSERT INTO public.audit_detail VALUES (1, 1), (2, 2);
INSERT INTO app.entries VALUES (1, 1), (2, 2);",
    );
    migrate_all(&f, "second.json");
    let rows = || -> Vec<i64> {
        let row = f
            .target()
            .query_one(
                "SELECT (SELECT count(*) FROM public.notes), (SELECT count(*) FROM public.audit_log), (SELECT count(*) FROM public.audit_detail), (SELECT count(*) FROM app.entries)",
                &[],
            )
            .unwrap();
        (0..4).map(|i| row.get(i)).collect()
    };
    let dependency = |r: &Value| {
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["code"] == "DB_EXTERNAL_DEPENDENCY")
            .unwrap_or_else(|| panic!("{r}"))
            .clone()
    };

    // Another application's table, which the history does not create, refers
    // to public.audit_log.
    f.target()
        .batch_execute(
            "CREATE TABLE public.notes (id int PRIMARY KEY, log int REFERENCES public.audit_log (id) ON DELETE CASCADE);
INSERT INTO public.notes VALUES (1, 1), (2, 2);",
        )
        .unwrap();
    f.sql(
        "0003_prune.sql",
        "DELETE FROM public.audit_log WHERE id = 2;",
    );
    let plan = f.plan("third.json");
    let outside = f.apply("third.json", plan["plan_digest"].as_str());
    assert_eq!(outside["outcome"], "fail", "{outside}");
    assert_eq!(
        dependency(&outside)["message"],
        "Incoming foreign keys from outside the managed scope block execution."
    );
    assert_eq!(rows(), [2, 2, 2, 2]);

    // A foreign key into the managed schema gets the same refusal.
    f.target()
        .batch_execute(
            "ALTER TABLE public.notes DROP CONSTRAINT notes_log_fkey;
CREATE TABLE public.notes_accounts (id int PRIMARY KEY, account int REFERENCES app.accounts (id));",
        )
        .unwrap();
    let plan = f.plan("third-managed.json");
    let managed = f.apply("third-managed.json", plan["plan_digest"].as_str());
    assert_eq!(managed["outcome"], "fail", "{managed}");
    assert_eq!(dependency(&managed), dependency(&outside));
    assert_eq!(rows(), [2, 2, 2, 2]);

    // With neither, the DELETE runs, and cascades only into tables the
    // history creates.
    f.target()
        .batch_execute("DROP TABLE public.notes_accounts")
        .unwrap();
    migrate_all(&f, "third-clear.json");
    assert_eq!(rows(), [2, 1, 1, 1]);
}

/// Objects a migration creates outside the managed scope without naming the
/// schema in a statement NTNT reads, through search_path, a DO block or SQL
/// sqlparser does not parse, are found by replay and inspected alike.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn objects_created_outside_through_search_path_do_or_unparsed_sql_are_inspected() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t (id int);
SET LOCAL search_path TO public;
CREATE TABLE via_search_path (id int);",
    );
    f.sql(
        "0002_do.sql",
        "DO $$ BEGIN CREATE TABLE public.via_do (id int); END $$;",
    );
    f.sql(
        "0003_unparsed.sql",
        "CREATE TABLE public.via_unparsed (id int) USING heap;",
    );
    let plan = f.plan("plan.json");
    assert_eq!(
        plan["plan"]["pending"][2]["analysis"]["unknowns"],
        serde_json::json!(["LOCKS_NOT_PREDICTED", "SQL_NOT_PARSED"]),
        "{plan}"
    );
    assert_eq!(
        plan["plan"]["execution"]["outside_objects"],
        serde_json::json!([
            "public.relation:via_do",
            "public.relation:via_search_path",
            "public.relation:via_unparsed"
        ]),
        "{plan}"
    );
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(f.command("verify")["outcome"], "pass");
    for table in ["via_search_path", "via_do", "via_unparsed"] {
        f.target()
            .batch_execute(&format!("ALTER TABLE public.{table} RENAME TO moved"))
            .unwrap();
        let r = f.command("verify");
        assert_eq!(r["outcome"], "fail", "{table}: {r}");
        assert!(
            drift_names(&r, &format!("public.relation:{table}"), "missing"),
            "{table}: {}",
            r["drift"]
        );
        // The renamed table is no longer one the history created.
        assert!(
            !r["drift"].to_string().contains("moved"),
            "{table}: {}",
            r["drift"]
        );
        f.target()
            .batch_execute(&format!("ALTER TABLE public.moved RENAME TO {table}"))
            .unwrap();
        assert_eq!(f.command("verify")["outcome"], "pass", "{table}");
    }
}

/// Histories that create nothing outside the managed scope inspect and plan
/// exactly as before: no outside_objects in the plan or the report.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn histories_that_create_nothing_outside_record_no_outside_objects() {
    for history in [
        "CREATE TABLE app.t (id int PRIMARY KEY); CREATE INDEX t_id ON app.t (id);",
        "CREATE TABLE app.t (id int); CREATE SCHEMA audit; CREATE TABLE audit.log (id int); REVOKE CREATE ON SCHEMA public FROM PUBLIC;",
    ] {
        let f = Fixture::new();
        f.sql("0001_initial.sql", history);
        let plan = f.plan("plan.json");
        let keys: Vec<&String> = plan["plan"]["execution"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(keys, ["format", "limits", "prefix_fingerprints"], "{history}");
        assert!(plan["target"].get("outside_objects").is_none(), "{history}");
        let applied = f.apply("plan.json", plan["plan_digest"].as_str());
        assert_eq!(applied["outcome"], "pass", "{applied}");
        let verified = f.command("verify");
        assert_eq!(verified["outcome"], "pass", "{verified}");
        assert!(verified["target"].get("outside_objects").is_none(), "{history}");
    }
}

/// A check runs after migrations whose managed objects refer to an object the
/// history created outside the managed scope: check purity inspects those
/// objects as plan, verify and migrate do (#420, review r4654889022).
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_run_after_managed_objects_refer_to_objects_created_outside() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE public.audit_log (id int PRIMARY KEY, msg text);
CREATE TABLE app.entries (id int PRIMARY KEY, log int REFERENCES public.audit_log (id));
CREATE VIEW app.recent AS SELECT id, msg FROM public.audit_log;",
    );
    f.sql("0002_seed.sql", "INSERT INTO app.entries VALUES (1, NULL);");
    std::fs::write(
        f.root.path().join("migrations/0002_seed.toml"),
        "[ensures]\nseeded = 'SELECT EXISTS (SELECT 1 FROM app.entries)'\n",
    )
    .unwrap();
    let plan = f.plan("checked.json");
    assert_eq!(
        plan["plan"]["execution"]["outside_objects"],
        serde_json::json!(["public.relation:audit_log"]),
        "{plan}"
    );
    let applied = f.apply("checked.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(applied["assertions"]["executed"], 1, "{applied}");
}

/// A check may read a managed view, and the views it can reach may call only
/// builtin non-volatile functions. That holds in every schema inspected: the
/// managed schema, a schema the history creates, and the objects the history
/// creates in other schemas (here `public`). A managed view over a view there
/// that calls a writing function is refused before any check runs, so the
/// function never writes; a chain of builtin-only views across the same
/// schemas passes (#420, review r4654895923).
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_refuse_views_that_reach_a_writing_function_in_any_inspected_schema() {
    for (create, s) in [("CREATE SCHEMA audit;\n", "audit"), ("", "public")] {
        let f = Fixture::new();
        // The writing function and what it writes exist from the first
        // migration on, so a refused second migration leaves them to inspect.
        // A sequence advance survives the rollback that a refusal causes.
        f.sql(
            "0001_initial.sql",
            &format!(
                "{create}CREATE TABLE {s}.calls (n bigint);
CREATE SEQUENCE {s}.call_count;
CREATE FUNCTION {s}.touch() RETURNS boolean VOLATILE LANGUAGE sql
  AS $$ INSERT INTO {s}.calls VALUES (nextval('{s}.call_count')) RETURNING true $$;
CREATE VIEW {s}.pure AS SELECT lower('A') = 'a' AS ok;
CREATE VIEW app.pure AS SELECT ok FROM {s}.pure;"
            ),
        );
        std::fs::write(
            f.root.path().join("migrations/0001_initial.toml"),
            "[ensures]\npure = 'SELECT EXISTS (SELECT 1 FROM app.pure WHERE ok)'\n",
        )
        .unwrap();
        f.sql(
            "0002_writes.sql",
            &format!(
                "CREATE VIEW {s}.writes AS SELECT {s}.touch() AS ok;
CREATE VIEW app.writes AS SELECT ok FROM {s}.writes;"
            ),
        );
        std::fs::write(
            f.root.path().join("migrations/0002_writes.toml"),
            "[ensures]\nwrites = 'SELECT EXISTS (SELECT 1 FROM app.writes WHERE ok)'\n",
        )
        .unwrap();
        let plan = f.plan("plan.json");
        assert_eq!(plan["outcome"], "pass", "{s}: {plan}");
        if s == "public" {
            let outside = names_in(&plan["plan"]["execution"]["outside_objects"]);
            for key in ["public.relation:writes", "public.routine:touch()"] {
                assert!(outside.contains(&key), "{key}: {outside:?}");
            }
        }
        let r = f.apply("plan.json", plan["plan_digest"].as_str());
        assert!(code(&r, "DB_CHECK_UNSUPPORTED"), "{s}: {r}");
        // The builtin-only chain was checked and committed; the check that
        // reads the writing chain never ran.
        assert_eq!(r["progress"][0]["state"], "committed", "{s}: {r}");
        assert_eq!(r["progress"][1]["state"], "failed", "{s}: {r}");
        assert_eq!(r["assertions"]["executed"], 1, "{s}: {r}");
        let row = f
            .target()
            .query_one(
                &format!(
                    "SELECT (SELECT count(*) FROM {s}.calls), (SELECT is_called FROM {s}.call_count)"
                ),
                &[],
            )
            .unwrap();
        assert_eq!(
            (row.get::<_, i64>(0), row.get::<_, bool>(1)),
            (0, false),
            "{s}: the function ran"
        );
    }
}

/// Check purity gates what a check can reach, not every inspected schema. A
/// check names only managed-schema relations and reaches others only through
/// the views among them. Views that call user routines in a schema the
/// history creates and in `public`, which no managed view reads, leave the
/// managed schema's checks to run.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_run_beside_views_no_check_can_reach() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE SCHEMA audit;
CREATE FUNCTION audit.label() RETURNS text IMMUTABLE LANGUAGE sql AS $$ SELECT 'audit' $$;
CREATE VIEW audit.labelled AS SELECT audit.label() AS label;
CREATE FUNCTION public.stamp() RETURNS text IMMUTABLE LANGUAGE sql AS $$ SELECT 'public' $$;
CREATE VIEW public.stamped AS SELECT public.stamp() AS stamp;
CREATE TABLE app.accounts (id int PRIMARY KEY);",
    );
    f.sql("0002_seed.sql", "INSERT INTO app.accounts VALUES (1);");
    std::fs::write(
        f.root.path().join("migrations/0002_seed.toml"),
        "[ensures]\nseeded = 'SELECT EXISTS (SELECT 1 FROM app.accounts)'\n",
    )
    .unwrap();
    let plan = f.plan("plan.json");
    let outside = names_in(&plan["plan"]["execution"]["outside_objects"]);
    for key in ["public.relation:stamped", "public.routine:stamp()"] {
        assert!(outside.contains(&key), "{key}: {outside:?}");
    }
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
}

/// What a check reaches through views is gated in any schema, not only in
/// the inspected ones. The fixture, not the history, puts a view chain that
/// ends in a writing function into `pg_catalog` of both databases: a covered
/// managed view may read there although no plan inspects it. A managed view
/// over the chain is refused before its check runs, so the function never
/// writes (a sequence advance would survive the rollback).
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn checks_refuse_views_that_reach_a_writing_function_outside_the_inspected_scope() {
    let f = Fixture::new();
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    // Checks lock what they read in SHARE mode, which a view grants with
    // UPDATE. Only a superuser writes a system catalog, so the function
    // runs as its owner.
    for database in [&f.target, &f.scratch] {
        config.dbname(database);
        let mut admin = config.connect(postgres::NoTls).unwrap();
        admin
            .batch_execute(
                "SET allow_system_table_mods = on;
CREATE TABLE pg_catalog.ntnt_test_calls (n bigint);
CREATE SEQUENCE pg_catalog.ntnt_test_call_count;
CREATE FUNCTION pg_catalog.ntnt_test_touch() RETURNS boolean VOLATILE SECURITY DEFINER LANGUAGE sql
  AS $$ INSERT INTO pg_catalog.ntnt_test_calls VALUES (nextval('pg_catalog.ntnt_test_call_count')) RETURNING true $$;
CREATE VIEW pg_catalog.ntnt_test_writes AS SELECT pg_catalog.ntnt_test_touch() AS ok;
CREATE VIEW pg_catalog.ntnt_test_relay AS SELECT ok FROM pg_catalog.ntnt_test_writes;
GRANT SELECT, UPDATE ON pg_catalog.ntnt_test_relay, pg_catalog.ntnt_test_writes TO PUBLIC;",
            )
            .unwrap();
    }
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id int PRIMARY KEY);",
    );
    std::fs::write(
        f.root.path().join("migrations/0001_initial.toml"),
        "[ensures]\nempty = 'SELECT NOT EXISTS (SELECT 1 FROM app.accounts)'\n",
    )
    .unwrap();
    f.sql(
        "0002_writes.sql",
        "CREATE VIEW app.writes AS SELECT ok FROM pg_catalog.ntnt_test_relay;",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_writes.toml"),
        "[ensures]\nwrites = 'SELECT EXISTS (SELECT 1 FROM app.writes WHERE ok)'\n",
    )
    .unwrap();
    let plan = f.plan("plan.json");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_CHECK_UNSUPPORTED"), "{r}");
    // The first migration's check ran and committed; the check that reads
    // the chain never ran.
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
    config.dbname(&f.target);
    let row = config
        .connect(postgres::NoTls)
        .unwrap()
        .query_one(
            "SELECT (SELECT count(*) FROM pg_catalog.ntnt_test_calls), (SELECT is_called FROM pg_catalog.ntnt_test_call_count)",
            &[],
        )
        .unwrap();
    assert_eq!(
        (row.get::<_, i64>(0), row.get::<_, bool>(1)),
        (0, false),
        "the function ran"
    );
}

/// #460: plans saved before restore-stable values fingerprinted values as
/// PostgreSQL stores them, and record canonicalization_version 1. Such a plan
/// for a history whose stored values a restore changes still passes
/// `verify --against` and migrates, from a target where part of the history
/// is applied, so its base and its later states are both in stored form. Its
/// fingerprints are the ones ntnt 91a7b9e (#458) recorded for this history;
/// only the stored form of each catalog matches them.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn plans_fingerprinting_values_as_stored_still_migrate() {
    let f = Fixture::new();
    f.sql(
        "0001_checks.sql",
        include_str!("fixtures/db_restore/0001_checks.sql"),
    );
    let first = f.plan("first.json");
    let r = f.apply("first.json", first["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    f.sql(
        "0002_partitions.sql",
        include_str!("fixtures/db_restore/0002_partitions.sql"),
    );
    f.plan("plan.json");
    let major: i32 = f
        .target()
        .query_one(
            "SELECT current_setting('server_version_num')::int / 10000",
            &[],
        )
        .unwrap()
        .get(0);
    let empty = "e0b5e359882940c7b4bc8008309032618468ec05ea2b92f0435e3d4ae3ec82d7";
    let stored = match major {
        16 => [
            empty,
            "ca8e5d115ffcab53308efc104bebaf22a3b23597cfdab79a084c4c4167e70e2e",
            "43b7d6b8ad0dfcabff3e8f5004f5789859fc6835ce0f3e45674468660cce3853",
        ],
        17 => [
            empty,
            "fd0e7fb81e8a58748f19c50c093c80d5fbc88e2ba1aeb770dda2ee93b2e34536",
            "6797963ab3e9e5fdaf01fe66378e79593854a2e47c925441fd484274cd1e4a07",
        ],
        18 => [
            empty,
            "e04d0d3b307f6254bf46ff20f2a6f9735c41071e521c7996645fa5564c34fb39",
            "1da48c526a2715ee7dd2c65ce4be2b427f9e6ebb69acf5ba514c6f5b8fc66f9e",
        ],
        other => panic!("no recorded fingerprints for PostgreSQL {other}"),
    };
    let mut saved: Value =
        serde_json::from_slice(&std::fs::read(f.root.path().join("plan.json")).unwrap()).unwrap();
    assert_eq!(plan_digest(&saved["plan"]), saved["plan_digest"]);
    let plan = &mut saved["plan"];
    assert_eq!(plan["binding"]["canonicalization_version"], 2, "{plan}");
    assert_eq!(plan["binding"]["applied_count"], 1, "{plan}");
    assert_eq!(plan["execution"]["prefix_fingerprints"][0], empty, "{plan}");
    assert_ne!(plan["binding"]["base_catalog_digest"], stored[1], "{plan}");
    plan["binding"]["canonicalization_version"] = 1.into();
    plan["binding"]["base_catalog_digest"] = stored[1].into();
    plan["binding"]["expected_base_digest"] = stored[1].into();
    plan["execution"]["prefix_fingerprints"] = stored.as_slice().into();
    plan["candidate"]["fingerprint"] = stored[2].into();
    let digest = plan_digest(&saved["plan"]);
    saved["plan_digest"] = Value::String(digest.clone());
    std::fs::write(
        f.root.path().join("stored.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    let checked = f.command_args("verify", &["--against", "stored.json"]);
    assert_eq!(checked["plan_validation"]["outcome"], "pass", "{checked}");
    let r = f.apply("stored.json", Some(&digest));
    assert_eq!(r["outcome"], "pass", "{r}");
    let verified = f.command("verify");
    assert_eq!(verified["outcome"], "pass", "{verified}");
}
