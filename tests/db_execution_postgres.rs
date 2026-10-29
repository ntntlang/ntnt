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
