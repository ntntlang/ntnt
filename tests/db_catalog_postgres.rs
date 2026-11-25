//! Real PostgreSQL contracts. Ignored locally unless explicitly selected; CI runs all
//! with a dedicated admin URL. Never use production credentials for this fixture.
use postgres::{Client, NoTls};
use serde_json::Value;
use std::process::Command;

#[path = "support/db_fixture.rs"]
mod db_fixture;
use db_fixture::Fixture;

/// Longest a test waits for a spawned `ntnt db` process to block on a
/// test-held lock.
const LOCK_WAIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn logical_database_and_target_selection_are_bound_into_plans() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let implicit = f.command("plan");
    let explicit = f.command_args("plan", &["--database", "app"]);
    assert_eq!(implicit["outcome"], "pass");
    assert_eq!(implicit["plan_digest"], explicit["plan_digest"]);
    assert_eq!(explicit["database"], "app");
    assert_eq!(explicit["binding"]["database"], "app");
    assert_eq!(explicit["binding"]["target_name"], "test");

    let manifest_path = f.root.path().join("ntnt.toml");
    let mut manifest: toml::Value = std::fs::read_to_string(&manifest_path)
        .unwrap()
        .parse()
        .unwrap();
    let alias = manifest["databases"]["app"].clone();
    manifest["databases"]
        .as_table_mut()
        .unwrap()
        .insert("alias".into(), alias);
    std::fs::write(manifest_path, toml::to_string(&manifest).unwrap()).unwrap();
    let app = f.command_args("plan", &["--database", "app", "--out", "app-plan.json"]);
    let alias = f.command_args("plan", &["--database", "alias"]);
    assert_eq!(app["outcome"], "pass");
    assert_eq!(alias["outcome"], "pass");
    assert_eq!(app["target"]["identity"], alias["target"]["identity"]);
    assert_ne!(app["plan_digest"], alias["plan_digest"]);
    let wrong = f.command_args(
        "verify",
        &["--database", "alias", "--against", "app-plan.json"],
    );
    assert_eq!(wrong["plan_validation"]["outcome"], "fail");
    let matching = f.command_args(
        "verify",
        &["--database", "app", "--against", "app-plan.json"],
    );
    assert_eq!(matching["plan_validation"]["outcome"], "pass");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn multiple_databases_have_independent_connections_migrations_and_scratch() {
    let app = Fixture::new();
    let analytics = Fixture::new();
    app.sql("0001_app.sql", "CREATE TABLE app.accounts(id INTEGER);");
    analytics
        .target()
        .batch_execute("CREATE SCHEMA analytics")
        .unwrap();
    let directory = app.root.path().join("analytics-migrations");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("0001_analytics.sql"),
        "CREATE TABLE analytics.events(id BIGINT);",
    )
    .unwrap();
    let manifest_path = app.root.path().join("ntnt.toml");
    let mut manifest: toml::Value = std::fs::read_to_string(&manifest_path)
        .unwrap()
        .parse()
        .unwrap();
    let other: toml::Value = std::fs::read_to_string(analytics.root.path().join("ntnt.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let mut definition = other["databases"]["app"].clone();
    definition["schema"] = "analytics".into();
    definition["migrations_dir"] = "analytics-migrations".into();
    definition["targets"]["test"]["connection_secret"] = "ANALYTICS_TARGET".into();
    definition["scratch"]["connection_secret"] = "ANALYTICS_SCRATCH".into();
    manifest["databases"]
        .as_table_mut()
        .unwrap()
        .insert("analytics".into(), definition);
    for secret in ["ANALYTICS_TARGET", "ANALYTICS_SCRATCH"] {
        manifest["secrets"]
            .as_table_mut()
            .unwrap()
            .insert(secret.into(), toml::Value::Table(Default::default()));
    }
    std::fs::write(manifest_path, toml::to_string(&manifest).unwrap()).unwrap();
    let run = |database: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args([
                "db",
                "plan",
                "--database",
                database,
                "--target",
                "test",
                "--json",
            ])
            .env_remove("DB_TARGET")
            .env_remove("DB_SCRATCH")
            .env_remove("ANALYTICS_TARGET")
            .env_remove("ANALYTICS_SCRATCH")
            .env(
                if database == "app" {
                    "DB_TARGET"
                } else {
                    "ANALYTICS_TARGET"
                },
                if database == "app" {
                    &app.target_url
                } else {
                    &analytics.target_url
                },
            )
            .env(
                if database == "app" {
                    "DB_SCRATCH"
                } else {
                    "ANALYTICS_SCRATCH"
                },
                if database == "app" {
                    &app.scratch_url
                } else {
                    &analytics.scratch_url
                },
            )
            .env("NTNT_ENV", "development")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .current_dir(app.root.path())
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success(), "{}", report["diagnostics"]);
        report
    };
    let first = run("app");
    let second = run("analytics");
    assert_eq!(first["target"]["identity"]["database"], app.target);
    assert_eq!(second["target"]["identity"]["database"], analytics.target);
    assert_eq!(first["scratch"]["identity"]["database"], app.scratch);
    assert_eq!(second["scratch"]["identity"]["database"], analytics.scratch);
    assert_eq!(first["source"]["migrations"][0]["source"], "0001_app.sql");
    assert_eq!(
        second["source"]["migrations"][0]["source"],
        "0001_analytics.sql"
    );
    assert_eq!(second["target"]["schema"], "analytics");
    assert_ne!(first["plan_digest"], second["plan_digest"]);
    // A damaged unselected history must not be read while planning app.
    std::fs::write(directory.join("0001_analytics.toml"), "unsupported = true").unwrap();
    assert_eq!(run("app")["plan_digest"], first["plan_digest"]);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn unknown_ledger_namespace_never_masquerades_as_known_history() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t (id INTEGER PRIMARY KEY);");
    f.target()
        .batch_execute("CREATE FUNCTION _ntnt.extra() RETURNS integer LANGUAGE SQL AS 'SELECT 1'")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["outcome"], "unknown", "{r}");
    assert!(
        r["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "DB_LEDGER_UNKNOWN"),
        "{r}"
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn human_view_renders_catalog_coverage_and_pending_hazards() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t(id INTEGER); CREATE INDEX id_idx ON app.t(id);",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "plan", "--target", "test"])
        .env("DB_TARGET", &f.target_url)
        .env("DB_SCRATCH", &f.scratch_url)
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(f.root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Catalog coverage"));
    assert!(text.contains("0001_initial.sql"));
    assert!(text.contains("INDEX_BUILD_LOCK_SCAN"));
    assert!(text.contains("not_run (0 executed)"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn catalog_reports_fingerprint_sql_literals_without_disclosing_them() {
    let f = Fixture::new();
    f.target().batch_execute("CREATE VIEW app.secret_definition AS SELECT 'postgres://hidden:very_private@localhost/db'::text AS connection").unwrap();
    let r = f.command("status");
    assert!(!r.to_string().contains("very_private"));
    assert!(r["catalog"]["objects"]["view:secret_definition"]["definition"]["sha256"].is_string());
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn quoted_object_components_cannot_collide_in_catalog_keys() {
    let f = Fixture::new();
    f.seed_first(r#"CREATE TABLE app."a.b" (c INTEGER); CREATE TABLE app.a ("b.c" INTEGER);"#);
    f.target()
        .batch_execute(r#"ALTER TABLE app.a ALTER COLUMN "b.c" TYPE TEXT"#)
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["drift"]["outcome"], "fail");
    assert_eq!(
        r["catalog"]["objects"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("column:"))
            .count(),
        2
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn plan_identity_changes_when_role_authority_changes() {
    let mut f = Fixture::new();
    let before = f.command("plan");
    assert_eq!(before["outcome"], "pass");
    f.admin
        .batch_execute(&format!("ALTER ROLE {} CREATEDB", f.target))
        .unwrap();
    let after = f.command("plan");
    assert_ne!(before["plan_digest"], after["plan_digest"]);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn unqualified_plain_sql_replays_in_managed_scope_with_builtin_resolution() {
    let f = Fixture::new();
    f.sql("0001_initial.sql","CREATE TABLE accounts (id INTEGER PRIMARY KEY, label TEXT); CREATE INDEX label_idx ON accounts(label); ALTER TABLE accounts ADD COLUMN active BOOLEAN DEFAULT true;");
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn plan_binding_rejects_changed_source_and_wrong_target() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t (id INTEGER);");
    let plan = f.command_args("plan", &["--out", "review.json"]);
    let fresh = f.command_args("verify", &["--against", "review.json"]);
    assert_eq!(fresh["plan_validation"]["outcome"], "pass", "{fresh}");
    f.sql("0001_initial.sql", "CREATE TABLE app.t (id BIGINT);");
    let changed = f.command_args("verify", &["--against", "review.json"]);
    assert_eq!(changed["plan_validation"]["outcome"], "fail", "{changed}");
    assert_ne!(plan["plan_digest"], f.command("plan")["plan_digest"]);
    assert_eq!(changed["outcome"], "fail");
    let other = Fixture::new();
    other.sql("0001_initial.sql", "CREATE TABLE app.t (id INTEGER);");
    std::fs::copy(
        f.root.path().join("review.json"),
        other.root.path().join("review.json"),
    )
    .unwrap();
    let wrong = other.command_args("verify", &["--against", "review.json"]);
    assert_eq!(wrong["plan_validation"]["outcome"], "fail", "{wrong}");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn empty_target_replays_pending_without_mutating_either_database() {
    let f = Fixture::new();
    f.sql("0001_tables.sql", "CREATE TABLE app.accounts (id INTEGER PRIMARY KEY, label TEXT NOT NULL DEFAULT 'quoted; value');");
    let r = f.command("plan");
    assert_eq!(r["history"]["applied_count"], 0, "{r}");
    assert_eq!(r["history"]["pending_count"], 1, "{r}");
    assert_eq!(r["drift"]["outcome"], "pass", "{r}");
    assert_eq!(r["plan"]["candidate"]["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["outcome"], "not_run");
    assert_eq!(r["plan"]["authorized"], false);
    let mut target = f.target();
    assert!(!target
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname='_ntnt')",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    assert!(!target.query_one("SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON c.relnamespace=n.oid WHERE n.nspname='app')",&[]).unwrap().get::<_,bool>(0));
    let mut scratch = Client::connect(&f.scratch_url, NoTls).unwrap();
    assert!(!scratch
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname='app')",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
}

fn has_code(r: &Value, code: &str) -> bool {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn pending_is_not_drift_but_live_schema_change_is() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t (id INTEGER PRIMARY KEY, quantity INTEGER CONSTRAINT positive CHECK (quantity>0)); CREATE INDEX quantity_idx ON app.t(quantity);");
    f.sql(
        "0002_add.sql",
        "ALTER TABLE app.t ADD COLUMN label TEXT DEFAULT 'pending';",
    );
    let clean = f.command("plan");
    assert_eq!(
        clean["drift"]["outcome"], "pass",
        "{}",
        clean["diagnostics"]
    );
    assert_eq!(clean["history"]["applied_count"], 1);
    assert_eq!(clean["history"]["pending_count"], 1);
    assert_eq!(clean["plan_digest"], f.command("plan")["plan_digest"]);
    f.target().batch_execute("ALTER TABLE app.t DROP CONSTRAINT positive; ALTER TABLE app.t ADD CONSTRAINT positive CHECK(quantity>0) NOT VALID; GRANT SELECT ON app.t TO PUBLIC").unwrap();
    let drift = f.command("verify");
    assert_eq!(
        drift["drift"]["outcome"], "fail",
        "{}",
        drift["diagnostics"]
    );
    assert_eq!(
        drift["catalog"]["objects"]["constraint:t.positive"]["validated"],
        false
    );
    let diff = drift["drift"]["differences"].as_array().unwrap();
    assert!(diff.iter().any(|d| d["object"] == "constraint:t.positive"));
    assert!(diff.iter().any(|d| d["object"] == "relation:t"));
    assert!(!diff.iter().any(|d| d["object"] == "column:t.label"));
    let filtered = f.command_args("verify", &["--object", "not_present"]);
    assert_eq!(filtered["catalog"]["objects"].as_object().unwrap().len(), 0);
    assert_eq!(filtered["binding"], drift["binding"]);
    assert_eq!(filtered["outcome"], "fail");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn failed_concurrent_index_is_inventoried_as_invalid() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t (id INTEGER); CREATE UNIQUE INDEX id_unique ON app.t(id);");
    let mut c = f.target();
    c.batch_execute("DROP INDEX app.id_unique; INSERT INTO app.t VALUES(1),(1)")
        .unwrap();
    assert!(c
        .batch_execute("CREATE UNIQUE INDEX CONCURRENTLY id_unique ON app.t(id)")
        .is_err());
    let r = f.command("verify");
    assert_eq!(r["drift"]["outcome"], "fail", "{}", r["diagnostics"]);
    assert_eq!(r["catalog"]["objects"]["index:id_unique"]["valid"], false);
    assert!(r["catalog"]["objects"]["relation:id_unique"]["grants"].is_null());
    assert!(r["drift"]["differences"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["object"] == "index:id_unique"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn catalog_detects_disabled_foreign_key_enforcement() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.parents (id INTEGER PRIMARY KEY); CREATE TABLE app.children (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES app.parents(id));");
    assert_eq!(f.command("verify")["outcome"], "pass");
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    config.dbname(&f.target);
    let mut admin = config.connect(NoTls).unwrap();
    admin
        .batch_execute("ALTER TABLE app.children DISABLE TRIGGER ALL")
        .unwrap();
    // Prove this is an enforcement change, not just cosmetic catalog metadata.
    f.target()
        .batch_execute("INSERT INTO app.children VALUES (1, 99)")
        .unwrap();
    let report = f.command("verify");
    assert_eq!(
        report["drift"]["outcome"], "fail",
        "disabled foreign-key enforcement was not detected as drift: {report}"
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn catalog_detects_inheritance_children_outside_managed_scope() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.parent (id INTEGER);");
    assert_eq!(f.command("verify")["outcome"], "pass");
    let mut target = f.target();
    target
        .batch_execute("CREATE SCHEMA external_data; CREATE TABLE external_data.child () INHERITS (app.parent); INSERT INTO external_data.child VALUES (42)")
        .unwrap();
    assert_eq!(
        target
            .query_one("SELECT count(*) FROM app.parent", &[])
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    let report = f.command("verify");
    assert_eq!(
        report["drift"]["outcome"], "unknown",
        "unsupported inheritance was not exposed: {report}"
    );
    assert!(report["catalog"]["uncovered"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("relation:parent")));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn history_rejects_edited_missing_divergent_and_dirty_prefixes() {
    let f = Fixture::new();
    let original = "CREATE TABLE app.t (id INTEGER);";
    f.seed_first(original);
    f.sql("0001_initial.sql", &(original.to_owned() + " -- edited"));
    assert!(has_code(&f.command("status"), "DB_HISTORY_EDITED"));
    f.sql("0001_initial.sql", original);
    std::fs::write(
        f.root.path().join("migrations/0001_initial.toml"),
        "intent_ids = [] # new metadata",
    )
    .unwrap();
    assert!(has_code(&f.command("status"), "DB_HISTORY_EDITED"));
    std::fs::remove_file(f.root.path().join("migrations/0001_initial.toml")).unwrap();
    std::fs::remove_file(f.root.path().join("migrations/0001_initial.sql")).unwrap();
    assert!(has_code(&f.command("status"), "DB_HISTORY_MISSING"));
    f.sql("0002_other.sql", original);
    assert!(has_code(&f.command("status"), "DB_HISTORY_DIVERGENT"));
    std::fs::remove_file(f.root.path().join("migrations/0002_other.sql")).unwrap();
    f.sql("0001_initial.sql", original);
    f.target()
        .batch_execute("UPDATE _ntnt.schema_migrations_v1 SET state='running'")
        .unwrap();
    assert!(has_code(&f.command("status"), "DB_HISTORY_DIRTY"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn scratch_actual_identity_collision_is_rejected_without_mutation() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t (id INTEGER);");
    let path = f.root.path().join("ntnt.toml");
    let manifest = std::fs::read_to_string(&path)
        .unwrap()
        .replace(&f.scratch, &f.target);
    std::fs::write(path, manifest).unwrap();
    f.scratch_url = format!("{} application_name=different_url_spelling", f.target_url);
    let r = f.command("plan");
    assert!(has_code(&r, "DB_SCRATCH_COLLISION"), "{}", r["diagnostics"]);
    assert_eq!(r["outcome"], "unknown");
    assert_eq!(f.target().query_one("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='app'",&[]).unwrap().get::<_,i64>(0),0);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn scratch_requires_matching_engine_settings_empty_scope_and_low_privilege() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t (id INTEGER);");
    f.admin
        .batch_execute(&format!(
            "ALTER DATABASE {} SET timezone='Europe/Paris'",
            f.scratch
        ))
        .unwrap();
    assert!(has_code(&f.command("plan"), "DB_SCRATCH_MISMATCH"));
    f.admin
        .batch_execute(&format!("ALTER DATABASE {} RESET timezone", f.scratch))
        .unwrap();
    let mut scratch = Client::connect(&f.scratch_url, NoTls).unwrap();
    scratch
        .batch_execute("CREATE TABLE public.canary(id int)")
        .unwrap();
    assert!(has_code(&f.command("plan"), "DB_SCRATCH_NOT_EMPTY"));
    assert!(scratch
        .query_one("SELECT to_regclass('public.canary') IS NOT NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
    scratch.batch_execute("DROP TABLE public.canary").unwrap();
    f.admin
        .batch_execute(&format!("ALTER ROLE {} CREATEDB", f.scratch))
        .unwrap();
    assert!(has_code(&f.command("plan"), "DB_SCRATCH_NOT_EMPTY"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn unsupported_pending_routines_never_run_and_do_not_hide_applied_verification() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    for (sql, expected) in [
        (
            "CREATE INDEX CONCURRENTLY ON app.t(id)",
            "DB_REPLAY_UNSUPPORTED",
        ),
        ("COMMIT; DROP TABLE app.t", "DB_REPLAY_UNSUPPORTED"),
        ("SET ROLE postgres", "DB_REPLAY_UNSUPPORTED"),
        // Replayed in scratch; the failure is reported without server text.
        (
            "DO $$ BEGIN RAISE EXCEPTION 'secret_payload'; END $$;",
            "DB_POSTGRES",
        ),
    ] {
        f.sql("0002_bad.sql", sql);
        let r = f.command("plan");
        assert!(has_code(&r, expected), "{sql}: {}", r["diagnostics"]);
        assert_eq!(r["drift"]["outcome"], "pass");
        assert_eq!(r["plan"]["candidate"]["outcome"], "unknown");
        assert_eq!(f.command("verify")["outcome"], "pass");
        assert!(!r.to_string().contains("secret_payload"));
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn ledger_views_rls_and_missing_privileges_are_never_executed_as_history() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    let mut c = f.target();
    c.batch_execute("ALTER TABLE _ntnt.schema_migrations_v1 ENABLE ROW LEVEL SECURITY; CREATE POLICY hidden ON _ntnt.schema_migrations_v1 USING(false)").unwrap();
    assert!(has_code(&f.command("verify"), "DB_LEDGER_UNKNOWN"));
    c.batch_execute("ALTER TABLE _ntnt.schema_migrations_v1 DISABLE ROW LEVEL SECURITY; DROP POLICY hidden ON _ntnt.schema_migrations_v1").unwrap();
    c.batch_execute(&format!(
        "REVOKE SELECT ON _ntnt.schema_migrations_v1 FROM {}",
        f.target
    ))
    .unwrap();
    assert!(has_code(&f.command("verify"), "DB_LEDGER_UNKNOWN"));
    c.batch_execute("DROP TABLE _ntnt.schema_migrations_v1; CREATE VIEW _ntnt.schema_migrations_v1 AS SELECT 1 AS sleeper FROM pg_sleep(30)").unwrap();
    let before = std::time::Instant::now();
    assert!(has_code(&f.command("verify"), "DB_LEDGER_UNKNOWN"));
    assert!(before.elapsed() < std::time::Duration::from_secs(4));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn partial_catalog_coverage_and_nonempty_unadopted_scope_are_not_passes() {
    let f = Fixture::new();
    f.target()
        .batch_execute("CREATE TABLE app.legacy(id integer)")
        .unwrap();
    let r = f.command("status");
    assert!(has_code(&r, "DB_UNRECOGNIZED_HISTORY"));
    assert_eq!(r["outcome"], "fail");
    f.target().batch_execute("DROP TABLE app.legacy").unwrap();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    f.target()
        .batch_execute("CREATE MATERIALIZED VIEW app.slow AS SELECT 1 AS sleeper")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["drift"]["outcome"], "unknown");
    assert!(has_code(&r, "DB_COVERAGE_INCOMPLETE"));
    assert!(r["catalog"]["uncovered"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("view:slow")));
    // The diagnostic itself names the uncovered target objects.
    let coverage = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_COVERAGE_INCOMPLETE")
        .unwrap();
    assert_eq!(coverage["objects"], r["catalog"]["uncovered"], "{r}");
    // A plain view is compared by definition: unexpected, never "unknown".
    f.target()
        .batch_execute("DROP MATERIALIZED VIEW app.slow; CREATE VIEW app.slow AS SELECT 1 AS sleeper FROM pg_sleep(30)")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["drift"]["outcome"], "fail", "{r}");
    assert!(has_code(&r, "DB_DRIFT"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn target_lock_wait_is_bounded_and_session_is_discarded() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    let mut locker = f.target();
    locker
        .batch_execute("BEGIN; LOCK TABLE _ntnt.schema_migrations_v1 IN ACCESS EXCLUSIVE MODE")
        .unwrap();
    let start = std::time::Instant::now();
    let r = f.command("verify");
    assert_ne!(r["outcome"], "pass");
    assert!(start.elapsed() < std::time::Duration::from_secs(8));
    locker.batch_execute("ROLLBACK; SET lock_timeout='1s'; ALTER TABLE _ntnt.schema_migrations_v1 ADD COLUMN canary integer").unwrap();
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn replay_consumes_captured_files_even_if_paths_are_replaced_during_inspection() {
    let mut f = Fixture::new();
    f.seed_first("CREATE TABLE app.t (id INTEGER);");
    f.sql("0002_add.sql", "ALTER TABLE app.t ADD COLUMN label TEXT;");
    let expected = f.command("plan");
    assert_eq!(expected["outcome"], "pass");
    let mut locker = f.target();
    locker
        .batch_execute("BEGIN; LOCK TABLE _ntnt.schema_migrations_v1 IN ACCESS EXCLUSIVE MODE")
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "plan", "--target", "test", "--json"])
        .env("DB_TARGET", &f.target_url)
        .env("DB_SCRATCH", &f.scratch_url)
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(f.root.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        let waiting: i64 = f
            .admin
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE usename=$1 AND wait_event_type='Lock'",
                &[&f.target],
            )
            .unwrap()
            .get(0);
        if waiting > 0 {
            break;
        }
        // The child must reach the lock: fail fast (with its output) if it
        // exits first. The deadline only bounds a hang; a loaded CI runner can
        // take well over 10 s to start, inspect the target and connect.
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "inspector did not reach the history lock: exited with {status}\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if start.elapsed() > LOCK_WAIT_DEADLINE {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "inspector did not reach the history lock within {LOCK_WAIT_DEADLINE:?}\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::remove_file(f.root.path().join("migrations/0002_add.sql")).unwrap();
    f.sql("0002_add.sql", "SELECT pg_sleep(30);");
    std::fs::write(f.root.path().join("ntnt.toml"), "database='replacement'").unwrap();
    locker.batch_execute("ROLLBACK").unwrap();
    let output = child.wait_with_output().unwrap();
    let r: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(r["outcome"], "pass", "{}", r["diagnostics"]);
    assert_eq!(r["plan_digest"], expected["plan_digest"]);
    assert!(f.target().query_one("SELECT NOT EXISTS(SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='app' AND c.relname='t' AND a.attname='label')",&[]).unwrap().get::<_,bool>(0));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn review_target_snapshot_is_released_before_scratch_replay() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    config.dbname(&f.scratch);
    let mut locker = config.connect(NoTls).unwrap();
    locker
        .batch_execute("BEGIN; LOCK TABLE pg_catalog.pg_namespace IN ACCESS EXCLUSIVE MODE")
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "plan", "--target", "test", "--json"])
        .env("DB_TARGET", &f.target_url)
        .env("DB_SCRATCH", &f.scratch_url)
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(f.root.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        let waiting: i64 = f
            .admin
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE usename=$1 AND wait_event_type='Lock'",
                &[&f.scratch],
            )
            .unwrap()
            .get(0);
        if waiting > 0 {
            break;
        }
        // The child must reach the lock: fail fast (with its output) if it
        // exits first. The deadline only bounds a hang; a loaded CI runner can
        // take well over 10 s to start, inspect the target and connect.
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "replay did not reach the controlled scratch lock: exited with {status}\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if start.elapsed() > LOCK_WAIT_DEADLINE {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "replay did not reach the controlled scratch lock within {LOCK_WAIT_DEADLINE:?}\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let target_transactions: i64 = f.admin.query_one(
        "SELECT count(*) FROM pg_stat_activity WHERE usename=$1 AND application_name='ntnt-db-inspect-v1' AND xact_start IS NOT NULL",
        &[&f.target],
    ).unwrap().get(0);
    locker.batch_execute("ROLLBACK").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(target_transactions, 0, "the read-only target transaction must end before scratch work; otherwise long replays hit its idle deadline");
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn review_large_report_saves_a_reopenable_plan() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let ids: Vec<_> = (0..6000)
        .map(|i| format!("requirement.{i:05}.{}", "x".repeat(80)))
        .collect();
    std::fs::write(
        f.root.path().join("migrations/0001_initial.toml"),
        format!("intent_ids = {ids:?}"),
    )
    .unwrap();
    let report = f.command_args("plan", &["--out", "large-plan.json"]);
    assert_eq!(report["outcome"], "pass");
    assert!(
        serde_json::to_vec_pretty(&report).unwrap().len() > 1_048_576,
        "fixture must reproduce a full report larger than the input bound"
    );
    let fresh = f.command_args("verify", &["--against", "large-plan.json"]);
    assert_eq!(
        fresh["plan_validation"]["outcome"], "pass",
        "a successfully saved plan must be readable without changing its source/target binding"
    );
    assert!(
        std::fs::metadata(f.root.path().join("large-plan.json"))
            .unwrap()
            .len()
            <= 1_048_576
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn review_external_extension_does_not_masquerade_as_corrupt_history() {
    let f = Fixture::new();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    f.target()
        .batch_execute("CREATE EXTENSION pgcrypto WITH SCHEMA public")
        .unwrap();
    let report = f.command("verify");
    assert_eq!(
        report["history"]["outcome"], "pass",
        "external catalog coverage is not ledger corruption"
    );
    assert_eq!(report["history"]["applied_count"], 1);
    assert_eq!(report["outcome"], "unknown");
    assert!(has_code(&report, "DB_COVERAGE_INCOMPLETE"));
    assert!(!has_code(&report, "DB_LEDGER_UNKNOWN"));
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn sql_errors_and_unrecognized_ledger_values_do_not_leak_literals() {
    let f = Fixture::new();
    f.sql(
        "0001_bad.sql",
        "CREATE TABLE app.t(id INTEGER DEFAULT 'ntnt245_private_literal');",
    );
    let r = f.command("plan");
    assert!(has_code(&r, "DB_POSTGRES"));
    assert!(!r.to_string().contains("ntnt245_private_literal"));
    std::fs::remove_file(f.root.path().join("migrations/0001_bad.sql")).unwrap();
    f.seed_first("CREATE TABLE app.t(id INTEGER);");
    f.target()
        .batch_execute("UPDATE _ntnt.schema_migrations_v1 SET name='ntnt245_private_ledger'")
        .unwrap();
    let r = f.command("status");
    assert!(has_code(&r, "DB_HISTORY_DIVERGENT"));
    assert!(!r.to_string().contains("ntnt245_private_ledger"));
}

fn diagnostic<'a>(r: &'a Value, code: &str) -> Option<&'a Value> {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
}

fn names(list: &Value) -> Vec<&str> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// #420: a schema the migration history creates belongs to the managed scope.
/// An out-of-band change there is drift on the named object, exactly as in
/// the managed schema, never a pass.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn objects_in_a_schema_the_history_creates_are_inspected() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id int PRIMARY KEY);\nCREATE SCHEMA audit;\nCREATE TABLE audit.log (id int PRIMARY KEY, msg text);\n",
    );
    let plan = f.plan("plan.json");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let clean = f.command("verify");
    assert_eq!(clean["outcome"], "pass", "{clean}");
    for (change, object) in [
        (
            "ALTER TABLE audit.log DROP COLUMN msg",
            "audit.column:log.msg",
        ),
        ("DROP TABLE audit.log", "audit.relation:log"),
    ] {
        f.target().batch_execute(change).unwrap();
        let r = f.command("verify");
        assert_eq!(r["outcome"], "fail", "{change}: {r}");
        assert!(has_code(&r, "DB_DRIFT"), "{change}: {r}");
        assert!(
            r["drift"]["differences"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["object"] == object && d["change"] == "missing"),
            "{change}: {}",
            r["drift"]["differences"]
        );
    }
    // Keys in a created schema are qualified; the managed schema's are not.
    let objects = clean["catalog"]["objects"].as_object().unwrap();
    for key in [
        "schema:app",
        "schema:audit",
        "relation:accounts",
        "audit.relation:log",
        "audit.column:log.msg",
        "audit.index:log_pkey",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    assert_eq!(
        clean["target"]["history_schemas"],
        serde_json::json!(["audit"])
    );
}

/// Coverage and privileges apply in a created schema as in the managed one.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn uncovered_objects_and_grants_in_a_created_schema_are_reported() {
    let f = Fixture::new();
    f.seed_first("CREATE SCHEMA audit; CREATE TABLE audit.log (id int);");
    assert_eq!(f.command("verify")["outcome"], "pass");
    f.target()
        .batch_execute("GRANT SELECT ON audit.log TO PUBLIC")
        .unwrap();
    let granted = f.command("verify");
    assert_eq!(granted["drift"]["outcome"], "fail", "{granted}");
    assert!(
        granted["drift"]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["object"] == "audit.relation:log" && d["change"] == "changed"),
        "{}",
        granted["drift"]["differences"]
    );
    f.target()
        .batch_execute("REVOKE SELECT ON audit.log FROM PUBLIC")
        .unwrap();
    assert_eq!(f.command("verify")["outcome"], "pass");
    f.target()
        .batch_execute("CREATE MATERIALIZED VIEW audit.summary AS SELECT 1 AS n")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["drift"]["outcome"], "unknown", "{r}");
    let coverage = diagnostic(&r, "DB_COVERAGE_INCOMPLETE").unwrap_or_else(|| panic!("{r}"));
    assert!(
        names(&coverage["objects"]).contains(&"audit.view:summary"),
        "{coverage}"
    );
    assert!(
        names(&r["catalog"]["uncovered"]).contains(&"audit.view:summary"),
        "{r}"
    );
}

/// Managed objects may refer to a schema the history creates: such foreign
/// keys, defaults, triggers and views are covered, a change in what they refer
/// to is drift, and same-named objects in both schemas keep distinct keys.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn references_between_schemas_of_the_scope_are_covered() {
    let f = Fixture::new();
    f.seed_first(
        "CREATE SCHEMA audit;
CREATE TABLE audit.log (id int PRIMARY KEY);
CREATE TABLE app.log (id int PRIMARY KEY);
CREATE FUNCTION audit.stamp() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE FUNCTION audit.next_id() RETURNS int LANGUAGE sql AS 'SELECT 1';
CREATE TABLE app.entries (id int DEFAULT audit.next_id(), log int REFERENCES audit.log (id));
CREATE TRIGGER entries_stamp BEFORE INSERT ON app.entries FOR EACH ROW EXECUTE FUNCTION audit.stamp();
CREATE VIEW app.logged AS SELECT e.id FROM app.entries e JOIN audit.log l ON l.id = e.log;",
    );
    let r = f.command("verify");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["catalog"]["uncovered"], serde_json::json!([]), "{r}");
    let objects = r["catalog"]["objects"].as_object().unwrap();
    for key in [
        "relation:log",
        "audit.relation:log",
        "constraint:entries.entries_log_fkey",
        "trigger:entries.entries_stamp",
        "view:logged",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    f.target()
        .batch_execute(
            "CREATE OR REPLACE FUNCTION audit.next_id() RETURNS int LANGUAGE sql AS 'SELECT 2'",
        )
        .unwrap();
    let d = f.command("verify");
    assert_eq!(d["outcome"], "fail", "{d}");
    assert!(
        d["drift"]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["object"] == "audit.routine:next_id()" && x["change"] == "changed"),
        "{}",
        d["drift"]
    );
}

/// Josh's decision on #420 (2026-11-21): statements that create, change,
/// drop, grant, revoke or comment on schemas or objects outside the managed
/// scope run as they did before #420. None is refused, whether it names the
/// other schema, reaches it through search_path or a DO block, or is SQL
/// NTNT does not parse, and planning never touches the target.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn statements_outside_the_managed_scope_are_not_refused() {
    let f = Fixture::new();
    // A shared schema and a table in public that exist on the target only.
    f.target()
        .batch_execute("CREATE SCHEMA shared; CREATE TABLE public.legacy (id int)")
        .unwrap();
    for sql in [
        "CREATE TABLE public.leak (id int);",
        // No-ops in scratch; on the target they act on public.legacy.
        "DROP TABLE IF EXISTS public.legacy;",
        "ALTER TABLE IF EXISTS public.legacy ADD COLUMN note text;",
        "GRANT SELECT ON ALL TABLES IN SCHEMA public TO PUBLIC;",
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC;",
        "GRANT USAGE ON SCHEMA public TO PUBLIC;",
        "COMMENT ON SCHEMA public IS 'shared';",
        "CREATE TABLE public.leak (id int); TRUNCATE public.leak; DROP TABLE public.leak;",
        "DO $$ BEGIN CREATE TABLE public.leak (id int); END $$;",
        "DO $$ BEGIN GRANT CREATE ON SCHEMA public TO PUBLIC; END $$;",
        "SET LOCAL search_path = public; CREATE TABLE leak (id int);",
        "SELECT pg_catalog.set_config('search_path', 'public', true); DROP TABLE IF EXISTS legacy;",
        // SQL sqlparser does not parse.
        "ALTER SEQUENCE IF EXISTS public.s RESTART WITH 5;",
        "DROP ROUTINE IF EXISTS public.cleanup();",
    ] {
        f.sql(
            "0001_initial.sql",
            &format!("CREATE TABLE app.t (id int);\n{sql}\n"),
        );
        let r = f.command("plan");
        assert_eq!(r["outcome"], "pass", "{sql}: {}", r["diagnostics"]);
        assert!(!has_code(&r, "DB_OUTSIDE_SCOPE"), "{sql}: {r}");
    }
    // A schema that exists only on the target is no refusal either: replay
    // fails for lack of it in scratch, as it did before #420.
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t (id int);\nCREATE TABLE shared.leak (id int);\n",
    );
    let r = f.command("plan");
    assert!(has_code(&r, "DB_POSTGRES"), "{r}");
    assert!(!has_code(&r, "DB_OUTSIDE_SCOPE"), "{r}");
    // Planning never touched the target's objects outside the scope.
    assert!(f
        .target()
        .query_one("SELECT to_regclass('public.legacy') IS NOT NULL AND to_regclass('public.leak') IS NULL AND to_regclass('shared.leak') IS NULL AND (SELECT count(*) FROM pg_attribute WHERE attrelid = 'public.legacy'::regclass AND attnum > 0) = 1", &[])
        .unwrap()
        .get::<_, bool>(0));
}

/// What would break NTNT's own guarantees stays refused: changes to `_ntnt`,
/// the schema that holds NTNT's ledger, and to PostgreSQL's system schemas.
/// Each statement is a no-op or fails in scratch, which has no `_ntnt` and
/// whose unprivileged role cannot change system schemas, yet would act on a
/// target, so it is refused by name before it runs; replay catches a `_ntnt`
/// that a DO block creates.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn changes_to_the_ledger_schema_and_system_schemas_are_refused() {
    let f = Fixture::new();
    for (sql, object) in [
        ("CREATE SCHEMA IF NOT EXISTS _ntnt;", "_ntnt"),
        (
            "DROP TABLE IF EXISTS _ntnt.schema_migrations_v2;",
            "_ntnt.schema_migrations_v2",
        ),
        (
            "ALTER TABLE IF EXISTS _ntnt.schema_migrations_v2 ADD COLUMN note text;",
            "_ntnt.schema_migrations_v2",
        ),
        // Unqualified names that resolve in _ntnt.
        (
            "SET LOCAL search_path TO _ntnt; ALTER TABLE IF EXISTS schema_migrations_v2 ADD COLUMN note text;",
            "_ntnt",
        ),
        (
            "SELECT set_config('search_path', '_ntnt', true); ALTER TABLE IF EXISTS schema_migrations_v2 ADD COLUMN note text;",
            "_ntnt",
        ),
        // SQL sqlparser does not parse.
        (
            "ALTER SEQUENCE IF EXISTS _ntnt.positions RESTART WITH 5;",
            "_ntnt.positions",
        ),
        ("GRANT USAGE ON SCHEMA pg_catalog TO PUBLIC;", "pg_catalog"),
        (
            "REVOKE SELECT ON ALL TABLES IN SCHEMA information_schema FROM PUBLIC;",
            "information_schema",
        ),
        // Statement text the analysis cannot see into: replay observes it.
        ("DO $$ BEGIN CREATE SCHEMA _ntnt; END $$;", "schema:_ntnt"),
    ] {
        f.sql(
            "0001_initial.sql",
            &format!("CREATE TABLE app.t (id int);\n{sql}\n"),
        );
        let r = f.command("plan");
        assert_ne!(r["outcome"], "pass", "{sql}: {r}");
        assert_eq!(r["plan"]["candidate"]["outcome"], "unknown", "{sql}: {r}");
        let refused = diagnostic(&r, "DB_OUTSIDE_SCOPE")
            .unwrap_or_else(|| panic!("{sql}: {}", r["diagnostics"]));
        assert_eq!(refused["source"], "0001_initial.sql", "{refused}");
        assert!(
            names(&refused["objects"]).contains(&object),
            "{sql}: {refused}"
        );
    }
}

/// Managed objects may refer to objects the history creates outside the
/// managed scope, as they may to a created schema: such foreign keys,
/// defaults, checks, indexes, triggers, policies and views are covered, and a
/// change to what they refer to is drift on the outside object.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn references_to_objects_the_history_creates_outside_are_covered() {
    let f = Fixture::new();
    f.seed_first(
        "CREATE TABLE public.audit_log (id int PRIMARY KEY);
CREATE FUNCTION public.stamp() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE FUNCTION public.next_id() RETURNS int LANGUAGE sql IMMUTABLE AS 'SELECT 1';
CREATE TABLE app.entries (id int DEFAULT public.next_id() CHECK (public.next_id() > 0), log int REFERENCES public.audit_log (id));
CREATE INDEX entries_next ON app.entries ((id + public.next_id()));
CREATE TRIGGER entries_stamp BEFORE INSERT ON app.entries FOR EACH ROW EXECUTE FUNCTION public.stamp();
ALTER TABLE app.entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY entries_all ON app.entries USING (public.next_id() = 1);
CREATE VIEW app.logged AS SELECT e.id FROM app.entries e JOIN public.audit_log l ON l.id = e.log;",
    );
    let r = f.command("verify");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["catalog"]["uncovered"], serde_json::json!([]), "{r}");
    assert_eq!(
        r["target"]["outside_objects"],
        serde_json::json!([
            "public.relation:audit_log",
            "public.routine:next_id()",
            "public.routine:stamp()"
        ]),
        "{r}"
    );
    let objects = r["catalog"]["objects"].as_object().unwrap();
    for key in [
        "public.relation:audit_log",
        "public.routine:next_id()",
        "public.routine:stamp()",
        "constraint:entries.entries_log_fkey",
        "index:entries_next",
        "trigger:entries.entries_stamp",
        "policy:entries.entries_all",
        "view:logged",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    f.target()
        .batch_execute(
            "CREATE OR REPLACE FUNCTION public.next_id() RETURNS int LANGUAGE sql IMMUTABLE AS 'SELECT 2'",
        )
        .unwrap();
    let d = f.command("verify");
    assert_eq!(d["outcome"], "fail", "{d}");
    assert!(
        d["drift"]["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["object"] == "public.routine:next_id()" && x["change"] == "changed"),
        "{}",
        d["drift"]
    );
}

/// An extension a migration installs outside the managed scope is not
/// covered, as before #420, and its members follow it: the diagnostic names
/// the extension, not each routine and type it put in public.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn an_extension_installed_outside_the_scope_is_uncovered_as_one_object() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.t (id int);\nCREATE EXTENSION hstore WITH SCHEMA public;\n",
    );
    let r = f.command("plan");
    assert_eq!(r["plan"]["candidate"]["outcome"], "unknown", "{r}");
    let coverage = diagnostic(&r, "DB_COVERAGE_INCOMPLETE").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(coverage["source"], "0001_initial.sql", "{coverage}");
    assert_eq!(
        names(&coverage["objects"]),
        ["extension:hstore"],
        "{coverage}"
    );
    assert!(!has_code(&r, "DB_OUTSIDE_SCOPE"), "{r}");
}

/// Two migrations of a single-schema history. It reaches every catalog query
/// whose SQL #420 changed.
const SINGLE_SCHEMA_HISTORY: [(&str, &str); 2] = [
    (
        "0001_initial.sql",
        "CREATE TABLE app.accounts (id integer PRIMARY KEY, email text NOT NULL UNIQUE, parent integer REFERENCES app.accounts(id), CHECK (id > 0));
CREATE INDEX accounts_email_lower ON app.accounts (lower(email)) WHERE email <> '';
CREATE TABLE app.events (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, account integer NOT NULL REFERENCES app.accounts(id) ON DELETE CASCADE);
CREATE VIEW app.account_emails AS SELECT id, email FROM app.accounts;
CREATE SEQUENCE app.counter;
CREATE FUNCTION app.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE TRIGGER accounts_touch BEFORE UPDATE ON app.accounts FOR EACH ROW EXECUTE FUNCTION app.touch();
ALTER TABLE app.accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY accounts_all ON app.accounts USING (true);
GRANT SELECT ON app.account_emails TO PUBLIC;
",
    ),
    (
        "0002_created.sql",
        "ALTER TABLE app.accounts ADD COLUMN created timestamp DEFAULT now();",
    ),
];

/// A history without CREATE SCHEMA keeps exactly the catalog it had before
/// #420, so plans saved by earlier releases keep validating. The
/// fingerprints were recorded with ntnt 0.6.4 before #420 (81a5ad0). They
/// differ between PostgreSQL major versions (for example the default table
/// privileges gained MAINTAIN in 17), never within one. #430 left them
/// unchanged too: it covers kinds of objects this history does not use.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn single_schema_catalog_fingerprints_are_unchanged() {
    let mut f = Fixture::new();
    for (name, sql) in SINGLE_SCHEMA_HISTORY {
        f.sql(name, sql);
    }
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    let major = f
        .admin
        .query_one(
            "SELECT current_setting('server_version_num')::int / 10000",
            &[],
        )
        .unwrap()
        .get::<_, i32>(0);
    let empty = "e0b5e359882940c7b4bc8008309032618468ec05ea2b92f0435e3d4ae3ec82d7";
    let expected = match major {
        16 => [
            empty,
            "fa27beb077d616c4dc1ba83c0f922432668c5f0a4501b2012a457eed579b878d",
            "91c98d8aac0941393b1983539e57a22461e93321e80f732059ae9b5e755f3615",
        ],
        17 => [
            empty,
            "ace2b9aae2336edacbe65862d184d70af85f9dc489ef785562e3c765807234a0",
            "10e838513cc36a4e69753c906b42a05152998f7e1d661ee321626edcdd849e5f",
        ],
        18 => [
            empty,
            "d47d0d362be207d3759c1947cdea743a9511573519bb83dae68f667990f482b3",
            "7f7835ca0326767f0776785a3ce3386a3ea435c79263524d13a11d4d258e7423",
        ],
        other => panic!("no recorded fingerprints for PostgreSQL {other}"),
    };
    assert_eq!(
        r["plan"]["execution"]["prefix_fingerprints"],
        serde_json::json!(expected),
        "PostgreSQL {major}"
    );
    // Nor did the plan or its binding gain a field.
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys(&r["plan"]),
        [
            "authorized",
            "automatic_safe",
            "binding",
            "candidate",
            "classification",
            "data_checks",
            "executable",
            "execution",
            "pending",
            "required_authority"
        ]
    );
    assert_eq!(
        keys(&r["plan"]["binding"]),
        [
            "adapter",
            "applied_count",
            "base_catalog_digest",
            "base_history_digest",
            "canonicalization_version",
            "database",
            "expected_base_digest",
            "format_version",
            "input_digest",
            "managed_schema",
            "policy_digest",
            "target",
            "target_name"
        ]
    );
    assert!(
        r["target"].get("history_schemas").is_none(),
        "{}",
        r["target"]
    );
}

/// #434: a policy whose expression calls a routine outside pg_catalog and the
/// managed scope depends on code `ntnt db` does not inspect, so it is
/// uncovered. The coverage query compared the policy's dependency with the
/// routine's own OID (the inner alias shadowed the policy's), so every policy
/// read as covered.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_policy_calling_a_routine_outside_the_scope_is_uncovered() {
    let f = Fixture::new();
    f.sql(
        "0001_rls.sql",
        "CREATE TABLE app.t(id int PRIMARY KEY); ALTER TABLE app.t ENABLE ROW LEVEL SECURITY; \
         CREATE POLICY t_read ON app.t USING (information_schema._pg_numeric_precision(23, -1) IS NOT NULL);",
    );
    let r = f.command("plan");
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_COVERAGE_INCOMPLETE")
        .unwrap_or_else(|| panic!("no DB_COVERAGE_INCOMPLETE in {r}"));
    assert!(
        d["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o.as_str().unwrap().contains("t_read")),
        "{d}"
    );
}

/// A difference of `change` on `object` in a verify report.
fn drifted(r: &Value, object: &str, change: &str) -> bool {
    r["drift"]["differences"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["object"] == object && d["change"] == change)
}

/// Plan, approve and migrate the history in `f`, then verify it.
fn migrate_and_verify(f: &Fixture) -> Value {
    let plan = f.plan("plan.json");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let clean = f.command("verify");
    assert_eq!(clean["outcome"], "pass", "{clean}");
    assert_eq!(
        clean["catalog"]["uncovered"],
        serde_json::json!([]),
        "{clean}"
    );
    clean
}

/// #430 (pg-05): enums, domains and composite types, in the managed schema
/// and in a schema the history creates, are compared by definition like
/// tables. A history that uses them plans, migrates and verifies, columns of
/// those types are covered, and an out-of-band change to a type is drift on
/// that type.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn enums_domains_and_composite_types_are_compared_by_definition() {
    let f = Fixture::new();
    f.sql(
        "0001_types.sql",
        "CREATE TYPE app.order_status AS ENUM ('pending', 'paid', 'shipped');
CREATE TABLE app.orders (id int PRIMARY KEY, status app.order_status NOT NULL DEFAULT 'pending', history app.order_status[]);
CREATE DOMAIN app.email AS text CHECK (VALUE ~ '@');
CREATE TABLE app.contacts (id int PRIMARY KEY, email app.email NOT NULL);
CREATE TYPE app.address AS (street text, city text);
CREATE TABLE app.places (id int PRIMARY KEY, addr app.address);
CREATE SCHEMA billing;
CREATE DOMAIN billing.cents AS bigint NOT NULL DEFAULT 0 CHECK (VALUE >= 0);
CREATE TABLE app.invoices (id int PRIMARY KEY, total billing.cents);
",
    );
    let clean = migrate_and_verify(&f);
    let objects = clean["catalog"]["objects"].as_object().unwrap();
    for key in [
        "type:order_status",
        "type:email",
        "relation:address",
        "column:address.street",
        "column:orders.status",
        "column:orders.history",
        "column:contacts.email",
        "column:places.addr",
        "billing.type:cents",
        "column:invoices.total",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    // Enum labels are literals: reported by digest, as definitions are.
    assert_eq!(
        objects["type:order_status"]["labels"]["redacted"], true,
        "{}",
        objects["type:order_status"]
    );
    for (change, object, how) in [
        (
            "ALTER TYPE app.order_status ADD VALUE 'refunded'",
            "type:order_status",
            "changed",
        ),
        (
            "ALTER DOMAIN app.email DROP CONSTRAINT email_check; ALTER DOMAIN app.email ADD CONSTRAINT email_check CHECK (VALUE ~ '@.')",
            "type:email",
            "changed",
        ),
        (
            "ALTER DOMAIN billing.cents SET DEFAULT 1",
            "billing.type:cents",
            "changed",
        ),
        (
            "ALTER TYPE app.address ADD ATTRIBUTE zip text",
            "column:address.zip",
            "unexpected",
        ),
        (
            "REVOKE USAGE ON TYPE app.address FROM PUBLIC",
            "relation:address",
            "changed",
        ),
    ] {
        f.target().batch_execute(change).unwrap();
        let r = f.command("verify");
        assert_eq!(r["outcome"], "fail", "{change}: {r}");
        assert!(drifted(&r, object, how), "{change}: {}", r["drift"]);
    }
}

/// #430 (pg-04): a trusted extension installed in the managed schema is
/// covered by its name and version, and so are its members: types,
/// operators, operator classes and families, aggregates and casts. Indexes
/// and columns that use them are covered, citext's implicit casts do not
/// stop later migrations, and changing the version out of band is drift.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn trusted_extensions_in_the_managed_schema_are_covered_by_version() {
    let f = Fixture::new();
    // The #430 repro, `SCHEMA` without `WITH` included.
    f.sql(
        "0001_trgm.sql",
        "CREATE EXTENSION pg_trgm SCHEMA app;
CREATE TABLE app.products (id int PRIMARY KEY, name text NOT NULL);
CREATE INDEX products_name_trgm ON app.products USING gin (name app.gin_trgm_ops);
",
    );
    f.sql(
        "0002_citext.sql",
        "CREATE EXTENSION citext WITH SCHEMA app VERSION '1.4';
CREATE TABLE app.users (id int PRIMARY KEY, email app.citext NOT NULL UNIQUE);
",
    );
    f.sql(
        "0003_nick.sql",
        "ALTER TABLE app.users ADD COLUMN nick app.citext;\n",
    );
    let clean = migrate_and_verify(&f);
    let objects = clean["catalog"]["objects"].as_object().unwrap();
    for key in [
        "extension:pg_trgm",
        "extension:citext",
        "type:gtrgm",
        "type:citext",
        "index:products_name_trgm",
        "index:users_email_key",
        "column:users.email",
        "column:users.nick",
        "routine:max(app.citext)",
        "other:operator.%(text,text)",
        "other:operator.=(app.citext,app.citext)",
        "other:operator_class.gin_trgm_ops USING gin",
        "other:operator_class.citext_ops USING btree",
        "other:operator_class.citext_ops USING hash",
        "other:operator_family.citext_ops USING btree",
        "other:operator_family.citext_ops USING hash",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    // The installer owns the members; the extension's version covers them.
    assert_eq!(objects["type:citext"]["extension"], "citext");
    assert!(objects["type:citext"]["owner"].is_null());
    f.target()
        .batch_execute("ALTER EXTENSION citext UPDATE TO '1.5'")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["outcome"], "fail", "{r}");
    assert!(drifted(&r, "extension:citext", "changed"), "{}", r["drift"]);
}

/// #430 (pg-03): an operator is keyed with its argument types, as a routine
/// is, so overloads no longer collide. One whose function is covered is
/// covered, and dropping an overload out of band is drift on that overload.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn overloaded_operators_have_distinct_keys() {
    let f = Fixture::new();
    f.sql(
        "0001_ops.sql",
        "CREATE FUNCTION app.eqi(a int, b int) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT a = b $$;
CREATE FUNCTION app.eqt(a text, b text) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT a = b $$;
CREATE OPERATOR app.=== (LEFTARG = int, RIGHTARG = int, FUNCTION = app.eqi);
CREATE OPERATOR app.=== (LEFTARG = text, RIGHTARG = text, FUNCTION = app.eqt);
",
    );
    let clean = migrate_and_verify(&f);
    let objects = clean["catalog"]["objects"].as_object().unwrap();
    for key in [
        "other:operator.===(integer,integer)",
        "other:operator.===(text,text)",
    ] {
        assert!(objects.contains_key(key), "{key}: {:?}", objects.keys());
    }
    f.target()
        .batch_execute("DROP OPERATOR app.=== (text, text)")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["outcome"], "fail", "{r}");
    assert!(
        drifted(&r, "other:operator.===(text,text)", "missing"),
        "{}",
        r["drift"]
    );
    assert!(
        !drifted(&r, "other:operator.===(integer,integer)", "missing"),
        "{}",
        r["drift"]
    );
}

/// #430 (pg-03): objects that still share a catalog key (a database-wide
/// default privilege and one in the managed schema, for the same role and
/// object type) make inspection ambiguous. The diagnostic names the key and
/// the migration that made them.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn colliding_catalog_keys_name_the_key_and_the_migration() {
    let f = Fixture::new();
    f.sql(
        "0001_acl.sql",
        "CREATE TABLE app.t (id int);
ALTER DEFAULT PRIVILEGES GRANT SELECT ON TABLES TO PUBLIC;
ALTER DEFAULT PRIVILEGES IN SCHEMA app GRANT SELECT ON TABLES TO PUBLIC;
",
    );
    let r = f.command("plan");
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_CATALOG_AMBIGUOUS").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(d["source"], "0001_acl.sql", "{d}");
    let key = format!("other:default_acl.{}.r", f.scratch);
    assert_eq!(names(&d["objects"]), [key.as_str()], "{d}");
    assert!(
        d["next_action"].as_str().unwrap().contains("objects field"),
        "{d}"
    );
}

/// #430 (pg-06): EXCLUDE constraints are covered, with built-in operator
/// classes and with an extension's (btree_gist) installed in the managed
/// schema, and dropping one out of band is drift.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn exclusion_constraints_are_covered() {
    let f = Fixture::new();
    f.sql(
        "0001_bookings.sql",
        "CREATE TABLE app.bookings (id int PRIMARY KEY, room int NOT NULL, during tstzrange NOT NULL,
  CONSTRAINT bookings_no_overlap EXCLUDE USING gist (during WITH &&));
CREATE EXTENSION btree_gist WITH SCHEMA app;
CREATE TABLE app.rooms (id int PRIMARY KEY, room int NOT NULL, during tstzrange NOT NULL,
  CONSTRAINT rooms_no_overlap EXCLUDE USING gist (room WITH =, during WITH &&));
",
    );
    let clean = migrate_and_verify(&f);
    let objects = clean["catalog"]["objects"].as_object().unwrap();
    for key in [
        "constraint:bookings.bookings_no_overlap",
        "constraint:rooms.rooms_no_overlap",
    ] {
        assert_eq!(objects[key]["kind"], "x", "{key}: {:?}", objects.keys());
    }
    f.target()
        .batch_execute("ALTER TABLE app.bookings DROP CONSTRAINT bookings_no_overlap")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["outcome"], "fail", "{r}");
    assert!(
        drifted(&r, "constraint:bookings.bookings_no_overlap", "missing"),
        "{}",
        r["drift"]
    );
}

/// What #430 leaves outside coverage is still named, with the migration
/// that made it: a range type is uncovered, and a custom implicit cast that
/// is not an extension's stops the next migration by name.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn range_types_and_custom_casts_are_refused_by_name() {
    let f = Fixture::new();
    f.sql(
        "0001_span.sql",
        "CREATE TYPE app.span AS RANGE (subtype = int4);\n",
    );
    let r = f.command("plan");
    assert_ne!(r["outcome"], "pass", "{r}");
    let coverage = diagnostic(&r, "DB_COVERAGE_INCOMPLETE").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(coverage["source"], "0001_span.sql", "{coverage}");
    assert!(
        names(&coverage["objects"]).contains(&"type:span"),
        "{coverage}"
    );
    f.sql(
        "0001_span.sql",
        "CREATE TYPE app.mood AS ENUM ('ok');
CREATE FUNCTION app.mood_text(app.mood) RETURNS text LANGUAGE sql IMMUTABLE AS $$ SELECT 'x' $$;
CREATE CAST (app.mood AS text) WITH FUNCTION app.mood_text(app.mood) AS IMPLICIT;
",
    );
    f.sql("0002_next.sql", "CREATE TABLE app.t (id int);\n");
    let r = f.command("plan");
    assert_ne!(r["outcome"], "pass", "{r}");
    let cast = diagnostic(&r, "DB_CHECK_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(cast["source"], "0002_next.sql", "{cast}");
    assert_eq!(names(&cast["objects"]), ["cast:app.mood AS text"], "{cast}");
}

/// The same history with `public` as the managed schema, as many
/// applications have. Recorded with the ntnt 0.6.4 release binary, before
/// #430 changed the catalog queries.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn single_schema_catalog_fingerprints_are_unchanged_in_public() {
    let mut f = Fixture::with_schema("public");
    for (name, sql) in SINGLE_SCHEMA_HISTORY {
        f.sql(name, &sql.replace("app.", "public."));
    }
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    let major = f
        .admin
        .query_one(
            "SELECT current_setting('server_version_num')::int / 10000",
            &[],
        )
        .unwrap()
        .get::<_, i32>(0);
    let empty = "66b18e7c326d3809ac2689e4e29d0f148deba1b3866af541cfdc416f8dbf7a1a";
    let expected = match major {
        16 => [
            empty,
            "84fc02b092c3138b2066703bfac35f3b192e87620b31b4ea43a5ec0abb8ac18c",
            "75550a7d9fe10d0b536fe0a2dc72fa76baf7421d8596ee132a0aa3b1a2c215ec",
        ],
        17 => [
            empty,
            "fdb8c30aaa50a087cb0c2d2071a79675b6afd733b83ec69e9aa075d3dd3edb98",
            "ebe510bef74dec28b5bd22bb3ba8db6ed07c9697992e4e3a4f410b6717b73db8",
        ],
        18 => [
            empty,
            "ac1a4983271ecb6d86b8593814d8e5ef5acb62dd945baf96498887ad501b6d8f",
            "8420a1d8f099ea3ffcddb15d476b40615991713e2b8d8206e791fb143724a329",
        ],
        other => panic!("no recorded fingerprints for PostgreSQL {other}"),
    };
    assert_eq!(
        r["plan"]["execution"]["prefix_fingerprints"],
        serde_json::json!(expected),
        "PostgreSQL {major}"
    );
}

/// #430: checks, adoption checks included, may read tables whose columns
/// use the types the history covers (an enum, a domain, a composite type,
/// an extension's type in the managed schema), while citext's casts exist.
/// They still run no routine the history wrote: comparing a domain's values
/// does not evaluate its constraint, which here would advance a sequence.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_read_tables_with_columns_of_covered_types() {
    let f = Fixture::new();
    f.sql(
        "0001_types.sql",
        "CREATE EXTENSION citext WITH SCHEMA app;
CREATE SEQUENCE app.calls;
CREATE FUNCTION app.valid(t text) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN PERFORM nextval('app.calls'); RETURN position('@' in t) > 0; END $$;
CREATE DOMAIN app.email AS text CHECK (app.valid(VALUE));
CREATE TYPE app.order_status AS ENUM ('pending', 'paid');
CREATE TYPE app.address AS (street text, city text);
CREATE TABLE app.orders (id int PRIMARY KEY, status app.order_status NOT NULL, contact app.email, login app.citext, addr app.address);
",
    );
    f.sql(
        "0002_seed.sql",
        "INSERT INTO app.orders VALUES (1, 'paid', 'a@b', 'A', ROW('x', 'y'));\n",
    );
    std::fs::write(
        f.root.path().join("migrations/0002_seed.toml"),
        "[ensures]\npaid = \"SELECT EXISTS (SELECT 1 FROM app.orders WHERE status = 'paid' AND contact = 'a@b' AND contact <> 'b' AND login IS NOT NULL AND addr IS NOT NULL)\"\n",
    )
    .unwrap();
    let plan = f.plan("plan.json");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
    // Only the seed INSERT coerced a value into the domain.
    let calls: i64 = f
        .target()
        .query_one("SELECT last_value FROM app.calls", &[])
        .unwrap()
        .get(0);
    assert_eq!(calls, 1);
    assert_eq!(f.command("verify")["outcome"], "pass");
}

/// The first migration of a #456 check-purity case: an authored function
/// that also advances a sequence, and a domain whose constraint calls it. A
/// sequence advance survives the rollback a refusal causes, so `is_called`
/// shows whether the function ran.
const CALLS_DOMAIN: &str = "CREATE SEQUENCE app.calls;
CREATE FUNCTION app.valid(t text) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN PERFORM nextval('app.calls'); RETURN position('@' in t) > 0; END $$;
CREATE DOMAIN app.email AS text CHECK (app.valid(VALUE));
";

/// Migrates `setup` after `CALLS_DOMAIN`, then a second migration whose
/// `ensures` holds `checks`. Returns the migrate report and whether
/// `app.valid` ran.
fn migrate_checks(setup: &str, checks: &[(&str, &str)]) -> (Value, bool) {
    let f = Fixture::new();
    f.sql("0001_types.sql", &format!("{CALLS_DOMAIN}{setup}"));
    f.sql("0002_checked.sql", "CREATE TABLE app.marker (id int);\n");
    let ensures: String = checks
        .iter()
        .map(|(name, sql)| format!("{name} = {sql:?}\n"))
        .collect();
    std::fs::write(
        f.root.path().join("migrations/0002_checked.toml"),
        format!("[ensures]\n{ensures}"),
    )
    .unwrap();
    let plan = f.plan("plan.json");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    let ran = f
        .target()
        .query_one("SELECT is_called FROM app.calls", &[])
        .unwrap()
        .get(0);
    (r, ran)
}

/// #456 review: `check` is refused by name before it runs, the message
/// holding each of `named`, and `app.valid` never ran.
fn assert_check_refused(setup: &str, check: &str, named: &[&str]) {
    let (r, ran) = migrate_checks(setup, &[("nested", check)]);
    let d = diagnostic(&r, "DB_CHECK_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
    let message = d["message"].as_str().unwrap();
    for name in named {
        assert!(message.contains(name), "{name}: {d}");
    }
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    assert_eq!(r["assertions"]["executed"].as_u64().unwrap_or(0), 0, "{r}");
    assert!(!ran, "app.valid ran: {r}");
}

/// #456 review: comparing a column with a literal converts the literal to
/// the column's type, and each element of an array goes through its type's
/// input, domain constraints included. So a check that can read an array of
/// a domain whose constraint calls a routine the history wrote is refused
/// by name; so is one through a domain over that array, a constraint that
/// calls a volatile builtin, and one that uses an operator the history
/// wrote.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_refuse_arrays_of_domains_whose_constraints_call_history_code() {
    assert_check_refused(
        "CREATE TABLE app.orders (id int, emails app.email[]);\n",
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE emails = '{a@b}')",
        &[
            "table app.orders has column emails of type app.email[]",
            "domain app.email",
            "app.valid(text) (a routine the history wrote)",
        ],
    );
    assert_check_refused(
        "CREATE DOMAIN app.emails AS app.email[];\nCREATE TABLE app.orders (id int, emails app.emails);\n",
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE emails = '{a@b}')",
        &["column emails of type app.emails", "domain app.email,"],
    );
    assert_check_refused(
        "CREATE DOMAIN app.ticket AS text CHECK (nextval('app.calls') > 0);\nCREATE TABLE app.orders (id int, tickets app.ticket[]);\n",
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE tickets = '{x}')",
        &["domain app.ticket", "nextval(regclass) (a volatile function)"],
    );
    assert_check_refused(
        "CREATE FUNCTION app.same(a int, b int) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN PERFORM nextval('app.calls'); RETURN a = b; END $$;
CREATE OPERATOR app.=== (LEFTARG = int, RIGHTARG = int, FUNCTION = app.same);
CREATE DOMAIN app.three AS int CHECK (VALUE OPERATOR(app.===) 3);
CREATE TABLE app.orders (id int, threes app.three[]);
",
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE threes = '{3}')",
        &["domain app.three", "app.same(integer,integer)"],
    );
}

/// #456 review: a composite's attributes go through their types' input too.
/// In an array a composite meets a literal like any element; as a column's
/// top-level type it meets one as an anonymous record, which PostgreSQL
/// rejects before any input runs, but the gate counts it anyway.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_refuse_composites_with_domains_whose_constraints_call_history_code() {
    let contact = "CREATE TYPE app.contact AS (email app.email, name text);\n";
    assert_check_refused(
        &format!("{contact}CREATE TABLE app.orders (id int, contact app.contact);\n"),
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE contact IS NULL)",
        &[
            "column contact of type app.contact",
            "domain app.email",
            "app.valid(text)",
        ],
    );
    assert_check_refused(
        &format!("{contact}CREATE TABLE app.orders (id int, contacts app.contact[]);\n"),
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE contacts = '{\"(a@b,x)\"}')",
        &["column contacts of type app.contact[]", "domain app.email"],
    );
}

/// #456 review: a top-level domain over such a composite is refused too.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_refuse_domains_over_composites_with_such_domains() {
    assert_check_refused(
        "CREATE TYPE app.contact AS (email app.email, name text);
CREATE DOMAIN app.known_contact AS app.contact CHECK (VALUE IS NOT NULL);
CREATE TABLE app.orders (id int, contact app.known_contact);
",
        "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE contact IS NULL)",
        &[
            "column contact of type app.known_contact",
            "domain app.email",
        ],
    );
}

/// #456 review: reading a view runs its conversions and operators. A view
/// that converts values to a domain whose constraint calls a routine the
/// history wrote, or that uses an operator the history wrote, is refused
/// by name, though the view calls no such function itself.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_refuse_views_that_convert_to_such_domains_or_use_history_operators() {
    let orders = "CREATE TABLE app.orders (id int, email text);\nINSERT INTO app.orders VALUES (1, 'a@b');\n";
    assert_check_refused(
        &format!("{orders}CREATE VIEW app.contacts AS SELECT id, email::app.email AS email FROM app.orders;\n"),
        "SELECT EXISTS (SELECT 1 FROM app.contacts WHERE email IS NOT NULL)",
        &[
            "view app.contacts converts values to app.email",
            "app.valid(text)",
        ],
    );
    assert_check_refused(
        &format!("{orders}CREATE FUNCTION app.same(a int, b int) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN PERFORM nextval('app.calls'); RETURN a = b; END $$;
CREATE OPERATOR app.=== (LEFTARG = int, RIGHTARG = int, FUNCTION = app.same);
CREATE VIEW app.firsts AS SELECT id FROM app.orders WHERE id OPERATOR(app.===) 1;
"),
        "SELECT EXISTS (SELECT 1 FROM app.firsts WHERE id IS NOT NULL)",
        &[
            "view app.firsts uses operator app.===(integer,integer)",
            "app.same(integer,integer) (a routine the history wrote)",
        ],
    );
    // The planner may put an operator's negator in its place.
    assert_check_refused(
        &format!("{orders}CREATE FUNCTION app.differ(a int, b int) RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN PERFORM nextval('app.calls'); RETURN a <> b; END $$;
CREATE OPERATOR app.!== (LEFTARG = int, RIGHTARG = int, FUNCTION = app.differ);
CREATE OPERATOR app.=== (LEFTARG = int, RIGHTARG = int, FUNCTION = int4eq, NEGATOR = !==);
CREATE VIEW app.others AS SELECT id FROM app.orders WHERE NOT (id OPERATOR(app.===) 2);
"),
        "SELECT EXISTS (SELECT 1 FROM app.others WHERE id IS NOT NULL)",
        &["which the planner may replace with app.!==(integer,integer)", "app.differ(integer,integer)"],
    );
    // Builtins that convert to a type given at run time record no
    // dependency on it.
    assert_check_refused(
        &format!("{orders}CREATE VIEW app.valid_emails AS SELECT id, pg_input_is_valid(email, 'app.email') AS ok FROM app.orders;\n"),
        "SELECT EXISTS (SELECT 1 FROM app.valid_emails WHERE ok)",
        &["view app.valid_emails calls pg_input_is_valid(text,text)"],
    );
    assert_check_refused(
        "CREATE TABLE app.people (id int, email app.email);
CREATE VIEW app.imports AS SELECT (json_populate_record(p, '{\"email\": \"a@b\"}')).email AS email FROM app.people p;
",
        "SELECT NOT EXISTS (SELECT 1 FROM app.imports WHERE email IS NULL)",
        &["view app.imports calls json_populate_record(anyelement,json,boolean)"],
    );
}

/// #456 review: what stays allowed. A literal compared with a column of a
/// domain, or of a domain over it, is converted to the base type, so the
/// constraint never runs, whatever it calls; a view may compare one too.
/// An array of a domain whose constraint uses only builtins (a regular
/// expression) is allowed: its elements' constraint runs and passes.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_run_with_plain_domains_and_builtin_only_nested_domains() {
    let (r, ran) = migrate_checks(
        "CREATE DOMAIN app.address AS text CHECK (VALUE ~ '@');
CREATE DOMAIN app.primary_email AS app.email;
CREATE TYPE app.copy AS (address app.address, name text);
CREATE TABLE app.orders (id int, contact app.email, backup app.primary_email, cc app.address[], copies app.copy[]);
CREATE VIEW app.recent AS SELECT id, contact FROM app.orders WHERE contact <> 'x';
",
        &[
            (
                "plain",
                "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE contact = 'a@b' OR backup = 'c@d')",
            ),
            (
                "regex",
                "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE cc = '{a@b}' OR copies = '{\"(c@d,x)\"}')",
            ),
            (
                "view",
                "SELECT NOT EXISTS (SELECT 1 FROM app.recent WHERE contact = 'a@b')",
            ),
        ],
    );
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 3, "{r}");
    assert!(!ran, "app.valid ran: {r}");
}

/// #456: the data of the tester's j3 case, a citext column holding
/// 'Admin@example.com', with a domain over citext, a citext array and an
/// enum beside it.
const CITEXT_USERS: &str = "CREATE EXTENSION citext WITH SCHEMA app;
CREATE DOMAIN app.email AS app.citext;
CREATE TYPE app.role AS ENUM ('admin', 'member');
CREATE TABLE app.users (id int PRIMARY KEY, email app.citext NOT NULL UNIQUE, contact app.email, aliases app.citext[], role app.role NOT NULL DEFAULT 'member');
INSERT INTO app.users VALUES (1, 'Admin@example.com', 'Admin@example.com', '{Admin@example.com}', 'admin');
";

/// Migrates `CITEXT_USERS` and `setup`, then a second migration whose
/// `ensures` holds `checks`. Returns the plan and the migrate report.
fn citext_checks(setup: &str, checks: &[(&str, &str)]) -> (Value, Value) {
    let f = Fixture::new();
    f.sql("0001_users.sql", &format!("{CITEXT_USERS}{setup}"));
    f.sql("0002_checked.sql", "CREATE TABLE app.marker (id int);\n");
    let ensures: String = checks
        .iter()
        .map(|(name, sql)| format!("{name} = {sql:?}\n"))
        .collect();
    std::fs::write(
        f.root.path().join("migrations/0002_checked.toml"),
        format!("[ensures]\n{ensures}"),
    )
    .unwrap();
    let plan = f.plan("plan.json");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    (plan, r)
}

/// #456: checks resolve operators with search_path=pg_catalog, so on the
/// j3 data `email = 'admin@example.com'` compared case-sensitive text and
/// passed where the application's citext `=` finds the account. Such a
/// comparison is refused by name before any check runs, and `plan` warns:
/// on a citext column, through a domain over citext as a scalar
/// subquery's output, on an enum the history gave its own `=`, on an
/// integer column the history gave an `=(integer,integer)` (an application
/// whose path lists `app` first uses it), and on a table's whole row
/// (`users = users`) under an `=` the history wrote for its row type.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_refuse_comparisons_the_application_makes_with_another_operator() {
    for (setup, check, named) in [
        (
            "",
            "SELECT NOT EXISTS (SELECT 1 FROM app.users WHERE email = 'admin@example.com')",
            &[
                "Check no_admin_account ([ensures]) compares column email of app.users (type app.citext) with =",
                "app.=(app.citext,app.citext)",
                "search_path=pg_catalog",
            ][..],
        ),
        (
            "",
            "SELECT (SELECT contact FROM app.users WHERE id = 1) <> 'admin@example.com'",
            &[
                "compares column contact of app.users (type app.email, a domain over app.citext) with <>",
                "app.<>(app.citext,app.citext)",
            ][..],
        ),
        (
            "CREATE FUNCTION app.role_eq(app.role, app.role) RETURNS boolean LANGUAGE sql IMMUTABLE AS 'SELECT $1::text = $2::text';
CREATE OPERATOR app.= (LEFTARG = app.role, RIGHTARG = app.role, FUNCTION = app.role_eq);
",
            "SELECT EXISTS (SELECT 1 FROM app.users WHERE role = 'admin')",
            &["compares column role of app.users (type app.role) with =", "app.=(app.role,app.role)"][..],
        ),
        (
            "CREATE FUNCTION app.int_eq(integer, integer) RETURNS boolean LANGUAGE sql IMMUTABLE AS 'SELECT $1 <> $2';
CREATE OPERATOR app.= (LEFTARG = integer, RIGHTARG = integer, FUNCTION = app.int_eq);
",
            "SELECT EXISTS (SELECT 1 FROM app.users WHERE id = 1)",
            &["compares column id of app.users (type integer) with =", "app.=(integer,integer)"][..],
        ),
        (
            "CREATE FUNCTION app.users_eq(app.users, app.users) RETURNS boolean LANGUAGE sql IMMUTABLE AS 'SELECT false';
CREATE OPERATOR app.= (LEFTARG = app.users, RIGHTARG = app.users, FUNCTION = app.users_eq);
",
            "SELECT NOT EXISTS (SELECT 1 FROM app.users WHERE NOT (users = users))",
            &["compares column users of app.users (type app.users) with =", "app.=(app.users,app.users)"][..],
        ),
    ] {
        let (plan, r) = citext_checks(setup, &[("no_admin_account", check)]);
        let warning = &plan["plan"]["check_warnings"][0];
        assert_eq!(warning["code"], "CHECK_OPERATOR_MISMATCH", "{plan}");
        assert_eq!(warning["source"], "0002_checked.sql", "{plan}");
        assert_eq!(warning["check"], "no_admin_account", "{plan}");
        assert_eq!(warning["relation"], "app.users", "{plan}");
        let d = diagnostic(&r, "DB_CHECK_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
        for name in named {
            assert!(d["message"].as_str().unwrap().contains(name), "{name}: {d}");
        }
        assert_eq!(d["objects"], serde_json::json!(["relation:users"]), "{d}");
        assert_eq!(d["source"], "0002_checked.sql", "{d}");
        assert!(
            d["next_action"].as_str().unwrap().contains("in a view that a migration creates"),
            "{d}"
        );
        assert_eq!(r["progress"][0]["state"], "committed", "{r}");
        assert_eq!(r["progress"][1]["state"], "failed", "{r}");
        assert_eq!(r["assertions"]["executed"].as_u64().unwrap_or(0), 0, "{r}");
    }
}

/// #456: a comparison on a builtin-typed column of a table that also has
/// citext columns, citext read with IS NULL, count(*) or GROUP BY, a citext
/// array (whose `=` compares elements with citext's default operator class
/// either way) and an enum compared with a literal all run, and pass.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_compare_builtin_columns_and_read_citext_without_comparing_it() {
    let (plan, r) = citext_checks(
        "",
        &[
            ("by_id", "SELECT EXISTS (SELECT 1 FROM app.users WHERE id = 1)"),
            ("no_null_email", "SELECT NOT EXISTS (SELECT 1 FROM app.users WHERE email IS NULL)"),
            ("one_user", "SELECT (SELECT count(*) FROM app.users) = 1"),
            (
                "unique_emails",
                "SELECT NOT EXISTS (SELECT email FROM app.users GROUP BY email HAVING count(*) > 1)",
            ),
            (
                "admin_alias",
                "SELECT EXISTS (SELECT 1 FROM app.users WHERE aliases = '{admin@example.com}')",
            ),
            ("has_admin", "SELECT EXISTS (SELECT 1 FROM app.users WHERE role = 'admin')"),
        ],
    );
    assert!(plan["plan"]["check_warnings"].is_null(), "{plan}");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 6, "{r}");
}

/// #456: the documented way to compare a citext column. A view a migration
/// creates keeps citext's `=`, so a check of the view sees what the
/// application sees and fails on the j3 data.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_view_keeps_the_application_operator_for_checks() {
    let (plan, r) = citext_checks(
        "CREATE VIEW app.admin_accounts AS SELECT id FROM app.users WHERE email = 'admin@example.com';\n",
        &[("no_admin_account", "SELECT NOT EXISTS (SELECT 1 FROM app.admin_accounts)")],
    );
    assert!(plan["plan"]["check_warnings"].is_null(), "{plan}");
    assert!(diagnostic(&r, "DB_CHECK_FAILED").is_some(), "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
    assert_eq!(r["assertions"]["checks"][0]["outcome"], "fail", "{r}");
}

/// #456: a check must not read a view, directly or through another view,
/// that calls a builtin that writes or takes locks. pg_depend records no
/// builtin, so the views' stored queries are read. The refusal names the
/// check, the view and the function, and the sequence shows the check never
/// ran: an advance survives the rollback.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_refuse_views_that_call_builtins_that_write_or_lock() {
    for (views, named) in [
        (
            "CREATE VIEW app.next_ids AS SELECT pg_catalog.nextval('app.ids') AS id;\n",
            &["Check ids ([ensures]) reads view app.next_ids, which calls nextval(regclass)"][..],
        ),
        (
            "CREATE VIEW app.inner_ids AS SELECT pg_catalog.nextval('app.ids') AS id;
CREATE VIEW app.next_ids AS SELECT id FROM app.inner_ids;
",
            &["reads view app.inner_ids, which calls nextval(regclass)"][..],
        ),
        (
            "CREATE VIEW app.next_ids AS SELECT pg_catalog.pg_try_advisory_xact_lock(1) AS locked, pg_catalog.setval('app.ids', 5) AS id;\n",
            &["pg_try_advisory_xact_lock(bigint)", "setval(regclass,bigint)"][..],
        ),
    ] {
        let f = Fixture::new();
        f.sql("0001_views.sql", &format!("CREATE SEQUENCE app.ids;\n{views}"));
        f.sql("0002_checked.sql", "CREATE TABLE app.marker (id int);\n");
        std::fs::write(
            f.root.path().join("migrations/0002_checked.toml"),
            "[ensures]\nids = \"SELECT (SELECT count(*) FROM app.next_ids) = 1\"\n",
        )
        .unwrap();
        let plan = f.plan("plan.json");
        assert_eq!(plan["plan"]["check_warnings"][0]["code"], "CHECK_VIEW_WRITES", "{plan}");
        let r = f.apply("plan.json", plan["plan_digest"].as_str());
        let d = diagnostic(&r, "DB_CHECK_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
        for name in named {
            assert!(d["message"].as_str().unwrap().contains(name), "{name}: {d}");
        }
        assert!(d["next_action"].as_str().unwrap().contains("underlying tables"), "{d}");
        assert_eq!(r["progress"][1]["state"], "failed", "{r}");
        assert_eq!(r["assertions"]["executed"].as_u64().unwrap_or(0), 0, "{r}");
        let called: bool = f
            .target()
            .query_one("SELECT is_called FROM app.ids", &[])
            .unwrap()
            .get(0);
        assert!(!called, "the check ran: {r}");
    }
}

/// #456: volatile builtins that change nothing stay allowed in views checks
/// read.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn checks_read_views_that_call_volatile_builtins_that_change_nothing() {
    let f = Fixture::new();
    f.sql(
        "0001_views.sql",
        "CREATE VIEW app.dice AS SELECT pg_catalog.random() AS r, pg_catalog.clock_timestamp() AS c, pg_catalog.now() AS t, pg_catalog.gen_random_uuid() AS u;\n",
    );
    std::fs::write(
        f.root.path().join("migrations/0001_views.toml"),
        "[ensures]\none_row = \"SELECT (SELECT count(*) FROM app.dice) = 1\"\n",
    )
    .unwrap();
    let plan = f.plan("plan.json");
    assert!(plan["plan"]["check_warnings"].is_null(), "{plan}");
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["assertions"]["executed"], 1, "{r}");
}

/// #456: PostgreSQL cannot use an enum value in the transaction that added
/// it (SQLSTATE 55P04). A migration that adds a value to an existing type
/// and uses it fails with DB_ENUM_VALUE_UNCOMMITTED naming the migration
/// and both statements. Replay commits each migration as the target does
/// (#430), so it fails at plan on every major, as `migrate` would; 17 and
/// later used to accept it in replay, whose one transaction had created
/// the type too.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn enum_values_used_in_the_transaction_that_added_them_are_named() {
    let f = Fixture::new();
    f.sql(
        "0001_orders.sql",
        "CREATE TYPE app.order_status AS ENUM ('pending', 'paid');
CREATE TABLE app.orders (id int PRIMARY KEY, status app.order_status NOT NULL DEFAULT 'pending');
",
    );
    f.sql(
        "0002_archived.sql",
        "ALTER TYPE app.order_status ADD VALUE 'archived';\nINSERT INTO app.orders VALUES (1, 'archived');\n",
    );
    let r = f.command("plan");
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_ENUM_VALUE_UNCOMMITTED").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(d["source"], "0002_archived.sql", "{d}");
    let message = d["message"].as_str().unwrap();
    assert!(message.starts_with("Statement 2 uses an enum value that ALTER TYPE ... ADD VALUE added in the same transaction (SQLSTATE 55P04). This migration adds an enum value in statement 1."), "{d}");
    let next = d["next_action"].as_str().unwrap();
    assert!(
        next.contains(
            "put ALTER TYPE ... ADD VALUE in its own migration and use the value in a later one"
        ),
        "{d}"
    );
    // Replay commits each migration (#430): splitting fixes plan on 16 too,
    // as enum_values_added_by_an_earlier_migration shows.
    assert!(
        next.contains("fixes plan and migrate alike, on PostgreSQL 16 too"),
        "{d}"
    );
    assert!(!next.contains("fixes migrate but not plan"), "{d}");
}

/// #456: a value added in one migration and used in a later one plans,
/// migrates and verifies on every major. Replay commits each migration as
/// the target does (#430); on 16 it used to fail with
/// DB_ENUM_VALUE_UNCOMMITTED, because its one transaction added the value.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn enum_values_added_by_an_earlier_migration() {
    let f = Fixture::new();
    f.sql(
        "0001_orders.sql",
        "CREATE TYPE app.order_status AS ENUM ('pending', 'paid');
CREATE TABLE app.orders (id int PRIMARY KEY, status app.order_status NOT NULL DEFAULT 'pending');
",
    );
    let first = f.plan("plan1.json");
    let r = f.apply("plan1.json", first["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    f.sql(
        "0002_archived.sql",
        "ALTER TYPE app.order_status ADD VALUE 'archived';\n",
    );
    f.sql(
        "0003_default.sql",
        "ALTER TABLE app.orders ALTER COLUMN status SET DEFAULT 'archived';\n",
    );
    let plan = f.plan("plan2.json");
    let r = f.apply("plan2.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify")["outcome"], "pass");
}
// #430: PostgreSQL replay commits each migration on scratch, in a transaction
// set up as the target's, then cleans scratch up. These tests drive the real
// CLI against the fixture's scratch database.

/// A superuser session on `f`'s scratch database.
fn scratch_admin(f: &Fixture) -> Client {
    let mut config: postgres::Config = std::env::var("NTNT_DB_TEST_ADMIN")
        .unwrap()
        .parse()
        .unwrap();
    config.dbname(&f.scratch);
    config.connect(NoTls).unwrap()
}

/// `public`'s owner, privileges and comment, in a scratch database.
const PUBLIC_STATE: &str = "SELECT pg_get_userbyid(nspowner)||' '||coalesce(nspacl::text,'-')||' '||coalesce(obj_description(oid,'pg_namespace'),'-') FROM pg_namespace WHERE nspname='public'";

fn public_state(f: &Fixture) -> String {
    scratch_admin(f)
        .query_one(PUBLIC_STATE, &[])
        .unwrap()
        .get(0)
}

/// What `f`'s scratch holds beyond the empty stock database it was: the
/// emptiness check's conditions (no schemas but `public`, nothing in it, no
/// extensions but plpgsql, no event triggers or default privileges), large
/// objects, publications, casts, languages and transforms, database and role
/// settings, privileges (compared as sets, so an explicit default is none)
/// and comments, the role's comment and memberships, and a `public` that
/// differs from `public_before`. Empty when scratch is clean.
fn scratch_leftovers(f: &Fixture, public_before: &str) -> Vec<String> {
    let sql = format!(
        r#"SELECT x FROM (
 SELECT 'schema '||nspname AS x FROM pg_namespace WHERE nspname NOT IN ('public','information_schema') AND nspname NOT LIKE 'pg\_%'
 UNION ALL SELECT 'relation public.'||c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'routine public.'||p.proname FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'
 UNION ALL SELECT 'type public.'||t.typname FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'operator public.'||o.oprname FROM pg_operator o JOIN pg_namespace n ON n.oid=o.oprnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'operator class public.'||o.opcname FROM pg_opclass o JOIN pg_namespace n ON n.oid=o.opcnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'operator family public.'||o.opfname FROM pg_opfamily o JOIN pg_namespace n ON n.oid=o.opfnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'collation public.'||c.collname FROM pg_collation c JOIN pg_namespace n ON n.oid=c.collnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'conversion public.'||c.conname FROM pg_conversion c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='public'
 UNION ALL SELECT 'text search configuration public.'||c.cfgname FROM pg_ts_config c JOIN pg_namespace n ON n.oid=c.cfgnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'text search dictionary public.'||c.dictname FROM pg_ts_dict c JOIN pg_namespace n ON n.oid=c.dictnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'text search parser public.'||c.prsname FROM pg_ts_parser c JOIN pg_namespace n ON n.oid=c.prsnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'text search template public.'||c.tmplname FROM pg_ts_template c JOIN pg_namespace n ON n.oid=c.tmplnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'statistics public.'||x.stxname FROM pg_statistic_ext x JOIN pg_namespace n ON n.oid=x.stxnamespace WHERE n.nspname='public'
 UNION ALL SELECT 'language '||lanname FROM pg_language WHERE oid>=16384
 UNION ALL SELECT 'transform '||trftype::regtype||' '||trflang FROM pg_transform WHERE oid>=16384
 UNION ALL SELECT 'role comment '||shobj_description(datdba,'pg_authid') FROM pg_database WHERE datname=current_database() AND shobj_description(datdba,'pg_authid') IS NOT NULL
 UNION ALL SELECT 'membership '||m.roleid::regrole||' '||m.member::regrole FROM pg_auth_members m, pg_database d WHERE d.datname=current_database() AND d.datdba IN (m.roleid, m.member)
 UNION ALL SELECT 'extension '||extname FROM pg_extension WHERE extname<>'plpgsql'
 UNION ALL SELECT 'event trigger '||evtname FROM pg_event_trigger
 UNION ALL SELECT 'default privileges '||oid FROM pg_default_acl
 UNION ALL SELECT 'large object '||oid FROM pg_largeobject_metadata
 UNION ALL SELECT 'publication '||pubname FROM pg_publication
 UNION ALL SELECT 'cast '||castsource::regtype||' AS '||casttarget::regtype FROM pg_cast WHERE oid>=16384
 UNION ALL SELECT 'setting '||array_to_string(s.setconfig, ',') FROM pg_db_role_setting s
   WHERE s.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database()) OR s.setrole=(SELECT datdba FROM pg_database WHERE datname=current_database())
 UNION ALL SELECT 'database '||datconnlimit||' '||coalesce(shobj_description(oid,'pg_database'),'-')||CASE WHEN acl THEN ' '||datacl::text ELSE '' END FROM (
   SELECT d.*, EXISTS (SELECT * FROM aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) EXCEPT SELECT * FROM aclexplode(acldefault('d', d.datdba)))
    OR EXISTS (SELECT * FROM aclexplode(acldefault('d', d.datdba)) EXCEPT SELECT * FROM aclexplode(coalesce(d.datacl, acldefault('d', d.datdba)))) AS acl
   FROM pg_database d WHERE d.datname=current_database()) d
   WHERE datconnlimit<>-1 OR shobj_description(oid,'pg_database') IS NOT NULL OR acl
 UNION ALL SELECT 'public '||state FROM ({PUBLIC_STATE}) p(state) WHERE state IS DISTINCT FROM $1
 UNION ALL SELECT 'no public' WHERE NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname='public')
) q ORDER BY 1"#
    );
    scratch_admin(f)
        .query(&sql, &[&public_before])
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
}

/// Run `plan` with `--out name`, approve and migrate it, then verify.
fn plan_migrate_verify(f: &Fixture, name: &str) {
    let plan = f.plan(name);
    let applied = f.apply(name, plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let verify = f.command("verify");
    assert_eq!(verify["outcome"], "pass", "{verify}");
}

/// Add `limits` to the fixture's manifest.
fn with_limits(f: &Fixture, limits: &str) {
    let path = f.root.path().join("ntnt.toml");
    let manifest = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        path,
        format!("{manifest}\n[databases.app.limits]\n{limits}\n"),
    )
    .unwrap();
}

fn spawn_plan(f: &Fixture) -> std::process::Child {
    f.process("plan")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

fn report(child: std::process::Child) -> Value {
    let output = child.wait_with_output().unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

/// Poll `query` (one boolean) on `c` until it holds. Fail with the child's
/// output if the child exits first; the deadline only bounds a hang.
fn wait_for(c: &mut Client, query: &str, child: &mut std::process::Child, what: &str) {
    let start = std::time::Instant::now();
    loop {
        if c.query_one(query, &[]).unwrap().get::<_, bool>(0) {
            return;
        }
        if child.try_wait().unwrap().is_some() {
            panic!("{what}: the plan exited first: {}", report_of(child));
        }
        if start.elapsed() > LOCK_WAIT_DEADLINE {
            let _ = child.kill();
            panic!("{what}: not reached within {LOCK_WAIT_DEADLINE:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// What an exited child wrote.
fn report_of(child: &mut std::process::Child) -> String {
    use std::io::Read;
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut out);
    }
    out
}

/// #430 (the tester's x_evo_enum stages): a value `ALTER TYPE ... ADD VALUE`
/// adds in one migration can be used by the next on every supported major.
/// PostgreSQL 16 forbids using it in the transaction that added it, which
/// used to be the whole replay. A migration that adds a value and uses it
/// in a column default fails at plan with DB_ENUM_VALUE_UNCOMMITTED, as the
/// target raises SQLSTATE 55P04, also on 17 and 18, where a type created in
/// the same transaction used to hide it.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn an_enum_value_added_by_one_migration_can_be_used_by_the_next() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql(
        "0001_orders.sql",
        "CREATE TYPE app.order_status AS ENUM ('pending', 'paid', 'shipped');\nCREATE TABLE app.orders (id int PRIMARY KEY, status app.order_status NOT NULL DEFAULT 'pending');",
    );
    f.sql(
        "0002_refunded.sql",
        "ALTER TYPE app.order_status ADD VALUE 'refunded';",
    );
    plan_migrate_verify(&f, "plan-1.json");
    let same = "ALTER TYPE app.order_status ADD VALUE 'archived' AFTER 'shipped';\nALTER TABLE app.orders ALTER COLUMN status SET DEFAULT 'archived';";
    f.sql("0003_archived_default.sql", same);
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_ENUM_VALUE_UNCOMMITTED").unwrap_or_else(|| panic!("{r}"));
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("This migration adds an enum value in statement 1."),
        "{d}"
    );
    assert_eq!(d["source"], "0003_archived_default.sql", "{d}");
    assert_ne!(r["outcome"], "pass", "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    // The target refuses the same script in a migration's transaction.
    let mut target = f.target();
    let mut tx = target.transaction().unwrap();
    let error = tx.batch_execute(same).unwrap_err();
    assert_eq!(error.code().map(|c| c.code()), Some("55P04"), "{error}");
    drop(tx);
    std::fs::remove_file(f.root.path().join("migrations/0003_archived_default.sql")).unwrap();
    f.sql(
        "0003_archived.sql",
        "ALTER TYPE app.order_status ADD VALUE 'archived' AFTER 'shipped';",
    );
    f.sql(
        "0004_archived_default.sql",
        "ALTER TABLE app.orders ALTER COLUMN status SET DEFAULT 'archived';",
    );
    plan_migrate_verify(&f, "plan-2.json");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430: a deferred constraint a migration violates fails that migration at
/// plan, as its commit fails on the target.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_deferred_constraint_violation_fails_at_plan() {
    let f = Fixture::new();
    let public = public_state(&f);
    let create = "CREATE TABLE app.codes (code text, CONSTRAINT codes_unique UNIQUE (code) DEFERRABLE INITIALLY DEFERRED);";
    let seed = "INSERT INTO app.codes VALUES ('a'), ('a');";
    f.sql("0001_codes.sql", create);
    f.sql("0002_seed.sql", seed);
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_POSTGRES").unwrap_or_else(|| panic!("{r}"));
    assert!(
        d["message"].as_str().unwrap().contains("SQLSTATE 23505"),
        "{d}"
    );
    assert_eq!(d["source"], "0002_seed.sql", "{d}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    let mut target = f.target();
    target.batch_execute(create).unwrap();
    let error = target
        .batch_execute(&format!("BEGIN; {seed} COMMIT"))
        .unwrap_err();
    assert_eq!(error.code().map(|c| c.code()), Some("23505"), "{error}");
}

/// #430: whatever a plan's outcome, scratch is as empty afterwards as before:
/// after success, after an error in a later migration, a refusal or a
/// coverage failure, with objects of the kinds a history can create as the
/// scratch role (in schemas it creates and in `public`), and after changes
/// to `public` itself, which cleanup restores, renamed or dropped included.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn scratch_is_empty_after_every_plan() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int PRIMARY KEY);");
    for (sql, expected) in [
        (
            "CREATE TYPE app.mood AS ENUM ('ok'); CREATE DOMAIN app.positive AS int CHECK (VALUE > 0);
             CREATE TYPE app.pair AS (a int, b text); CREATE SEQUENCE app.s; SELECT nextval('app.s');
             CREATE FUNCTION app.f() RETURNS int LANGUAGE sql AS 'SELECT 1';
             ALTER TABLE app.t ENABLE ROW LEVEL SECURITY; CREATE POLICY p ON app.t TO CURRENT_USER USING (true);
             CREATE EXTENSION citext WITH SCHEMA app; CREATE EXTENSION pg_trgm WITH SCHEMA app;
             CREATE EXTENSION btree_gist WITH SCHEMA app;
             CREATE TABLE public.audit (id int, mood app.mood);",
            Some("pass"),
        ),
        ("CREATE TABLE app.u (id int); SELECT 1/0;", Some("DB_POSTGRES")),
        (
            "DO $$ BEGIN EXECUTE 'CREATE SCHEMA _ntnt'; END $$;",
            Some("DB_OUTSIDE_SCOPE"),
        ),
        (
            "CREATE TYPE app.span AS RANGE (subtype = int4);",
            Some("DB_COVERAGE_INCOMPLETE"),
        ),
        (
            "CREATE TYPE app.pair AS (a int, b text); CREATE CAST (app.pair AS text) WITH INOUT;
             SELECT pg_catalog.lo_create(0); CREATE PUBLICATION ntnt_replay_pub FOR TABLE app.t;
             ALTER DEFAULT PRIVILEGES IN SCHEMA app GRANT SELECT ON TABLES TO PUBLIC;
             CREATE TEMPORARY TABLE scratch_notes (id int) ON COMMIT PRESERVE ROWS;",
            None,
        ),
        (
            "GRANT CREATE ON SCHEMA public TO PUBLIC; COMMENT ON SCHEMA public IS 'changed by a migration';
             CREATE TABLE public.notes (id int);",
            Some("pass"),
        ),
        (
            "ALTER SCHEMA public RENAME TO old_public; CREATE SCHEMA public; CREATE TABLE public.fresh (id int);",
            None,
        ),
        ("DROP SCHEMA public;", None),
        (
            "ALTER SCHEMA public OWNER TO CURRENT_USER; CREATE TABLE public.mine (id int);",
            None,
        ),
    ] {
        f.sql("0002_case.sql", sql);
        let r = f.command("plan");
        match expected {
            Some("pass") => assert_eq!(r["outcome"], "pass", "{sql}: {r}"),
            Some(code) => assert!(has_code(&r, code), "{sql}: {r}"),
            None => {}
        }
        assert_eq!(r["scratch"]["cleanup"], "pass", "{sql}: {r}");
        assert_eq!(
            scratch_leftovers(&f, &public),
            Vec::<String>::new(),
            "{sql}: {r}"
        );
    }
}

/// #430: a replayed migration commits on scratch, so a change cleanup could
/// not undo is refused before it commits: a new password (also one an
/// exception block rolled back, which PostgreSQL still counts as a write),
/// an object given to another role, a session left
/// acting as another role, and NTNT's replay marker, however it is changed.
/// Scratch keeps its settings, so the next plan still matches the target.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn changes_that_outlive_scratch_objects_are_refused_before_commit() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    for (sql, code) in [
        ("DO $$ BEGIN EXECUTE 'ALTER ROLE CURRENT_USER PASSWORD ''changed'''; END $$;", "DB_REPLAY_UNSUPPORTED"),
        ("DO $$ BEGIN EXECUTE 'SET LOCAL ROLE pg_database_owner'; EXECUTE 'CREATE TABLE public.given (id int)'; EXECUTE 'RESET ROLE'; END $$;", "DB_REPLAY_UNSUPPORTED"),
        ("ALTER DEFAULT PRIVILEGES FOR ROLE pg_database_owner GRANT SELECT ON TABLES TO PUBLIC;", "DB_REPLAY_UNSUPPORTED"),
        ("DO $$ BEGIN EXECUTE 'SET ROLE pg_database_owner'; END $$;", "DB_REPLAY_UNSUPPORTED"),
        ("DO $$ BEGIN EXECUTE 'UPDATE _ntnt_replay.replay_v1 SET public_comment = ''x'''; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'DROP TABLE _ntnt_replay.replay_v1'; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DELETE FROM _ntnt_replay.replay_v1;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN BEGIN EXECUTE 'ALTER ROLE CURRENT_USER PASSWORD ''changed'''; RAISE EXCEPTION 'undo'; EXCEPTION WHEN raise_exception THEN NULL; END; END $$;", "DB_REPLAY_UNSUPPORTED"),
        // NTNT's marker, changed in other ways than UPDATE, DELETE or DROP.
        ("DO $$ BEGIN EXECUTE 'TRUNCATE _ntnt_replay.replay_v1'; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'INSERT INTO _ntnt_replay.replay_v1 (id) VALUES (''other'')'; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'ALTER TABLE _ntnt_replay.replay_v1 ADD COLUMN extra int'; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'UPDATE _ntnt_replay.replay_v1 SET xids = ''{}'''; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'CREATE TABLE _ntnt_replay.extra (id int)'; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'ALTER SCHEMA _ntnt_replay RENAME TO elsewhere'; END $$;", "DB_OUTSIDE_SCOPE"),
        ("DO $$ BEGIN EXECUTE 'ALTER TABLE _ntnt_replay.replay_v1 ENABLE ROW LEVEL SECURITY'; END $$;", "DB_OUTSIDE_SCOPE"),
    ] {
        f.sql("0002_case.sql", sql);
        let r = f.command("plan");
        let d = diagnostic(&r, code).unwrap_or_else(|| panic!("{sql}: {r}"));
        assert_eq!(d["source"], "0002_case.sql", "{sql}: {d}");
        if sql.contains("RAISE EXCEPTION 'undo'") {
            assert!(
                d["message"].as_str().unwrap().contains("rolled it back"),
                "{d}"
            );
        }
        assert_eq!(r["scratch"]["cleanup"], "pass", "{sql}: {r}");
        assert_eq!(
            scratch_leftovers(&f, &public),
            Vec::<String>::new(),
            "{sql}: {r}"
        );
    }
    std::fs::remove_file(f.root.path().join("migrations/0002_case.sql")).unwrap();
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
}

/// #430: with `public` as the managed schema, as in Jarri's history, a
/// migration may grant on it, comment on it or take it over; cleanup gives
/// it back its owner, privileges and comment, so every plan starts from the
/// same catalog and gets the same digest, and the plan migrates.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn public_is_restored_after_replay() {
    let f = Fixture::with_schema("public");
    let public = public_state(&f);
    f.sql(
        "0001_accounts.sql",
        "CREATE TABLE accounts (id int PRIMARY KEY); GRANT CREATE ON SCHEMA public TO PUBLIC; COMMENT ON SCHEMA public IS 'accounts';",
    );
    f.sql(
        "0002_owner.sql",
        "ALTER SCHEMA public OWNER TO CURRENT_USER;",
    );
    let first = f.command("plan");
    assert_eq!(first["outcome"], "pass", "{first}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    assert_eq!(first["plan_digest"], f.command("plan")["plan_digest"]);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    plan_migrate_verify(&f, "plan.json");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// Run a plan whose 0002 waits for advisory lock 4242, which this holds,
/// once 0001 has committed on scratch; then kill it, as a crash would, and
/// end its backend.
fn crash_during_replay(f: &Fixture) {
    let mut holder = scratch_admin(f);
    holder
        .batch_execute("SELECT pg_advisory_lock(4242)")
        .unwrap();
    let mut child = spawn_plan(f);
    let waiting = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=4242 AND objsubid=1 AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted)";
    wait_for(&mut holder, waiting, &mut child, "replay waiting in 0002");
    let pid: i32 = holder
        .query_one("SELECT pid FROM pg_locks WHERE locktype='advisory' AND objid=4242 AND objsubid=1 AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted", &[])
        .unwrap()
        .get(0);
    child.kill().unwrap();
    child.wait().unwrap();
    holder
        .execute("SELECT pg_terminate_backend($1)", &[&pid])
        .unwrap();
    let start = std::time::Instant::now();
    while holder
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid=$1)",
            &[&pid],
        )
        .unwrap()
        .get::<_, bool>(0)
    {
        assert!(
            start.elapsed() < LOCK_WAIT_DEADLINE,
            "backend {pid} did not end"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    holder
        .batch_execute("SELECT pg_advisory_unlock(4242)")
        .unwrap();
}

/// #430: a migration that sets the scratch database's connection limit to 0
/// is refused before it commits. Cleanup would restore the limit, but a
/// replay stopped before cleanup would leave the scratch role unable to
/// connect, so no later plan could recover scratch. Another limit commits and
/// is restored by cleanup, as before.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_migration_that_closes_scratch_to_connections_is_refused_before_it_commits() {
    let f = Fixture::new();
    let public = public_state(&f);
    let limit = |f: &Fixture| -> i32 {
        scratch_admin(f)
            .query_one(
                "SELECT datconnlimit FROM pg_database WHERE datname=current_database()",
                &[],
            )
            .unwrap()
            .get(0)
    };
    f.sql(
        "0001_limit.sql",
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I CONNECTION LIMIT 0', current_database()); END $$;",
    );
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_REPLAY_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(d["source"], "0001_limit.sql", "{d}");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("connection limit to 0"),
        "{d}"
    );
    assert_eq!(r["scratch"]["cleanup"], "pass", "{r}");
    assert_eq!(limit(&f), -1);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    f.sql(
        "0001_limit.sql",
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I CONNECTION LIMIT 5', current_database()); END $$;",
    );
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["scratch"]["cleanup"], "pass", "{r}");
    assert_eq!(limit(&f), -1);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430: a migration that leaves the scratch role without CONNECT on scratch
/// is refused before it commits: one that revokes the role's own grant, and
/// one that revokes PUBLIC's when the role connects through it. Cleanup would
/// restore the privileges, but a replay stopped before cleanup would leave
/// the scratch role unable to connect, so no later plan could recover
/// scratch. A change that keeps the role connectable commits and cleanup
/// restores the privileges the DBA set, in one transaction that starts by
/// revoking them all.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_migration_that_revokes_the_scratch_roles_connect_is_refused_before_it_commits() {
    let revoke_own = "DO $$ BEGIN EXECUTE format('REVOKE CONNECT ON DATABASE %I FROM %I', current_database(), current_user); END $$;";
    let revoke_public = "DO $$ BEGIN EXECUTE format('REVOKE CONNECT ON DATABASE %I FROM PUBLIC', current_database()); END $$;";
    let grant_public = "DO $$ BEGIN EXECUTE format('GRANT CONNECT ON DATABASE %I TO PUBLIC', current_database()); END $$;";
    let grant_own = "DO $$ BEGIN EXECUTE format('GRANT CONNECT ON DATABASE %I TO %I', current_database(), current_user); END $$;";
    // (what the DBA set, the refused migration, what it names, a migration
    // that keeps the role connectable)
    let cases = [
        (
            "REVOKE CONNECT ON DATABASE {db} FROM PUBLIC",
            revoke_own,
            "the scratch role ",
            format!("{grant_public}\n{revoke_own}"),
        ),
        (
            "REVOKE CONNECT ON DATABASE {db} FROM {db}",
            revoke_public,
            "PUBLIC",
            format!("{grant_own}\n{revoke_public}"),
        ),
    ];
    for (baseline, refused, named, kept) in cases {
        let f = Fixture::new();
        let public = public_state(&f);
        scratch_admin(&f)
            .batch_execute(&baseline.replace("{db}", &f.scratch))
            .unwrap();
        // Cleanup restores privileges as a set, so compare them as one; the
        // rest of what scratch holds must be as it was.
        let acl = |f: &Fixture| -> Option<String> {
            scratch_admin(f)
                .query_one(
                    "SELECT (SELECT string_agg(a::text, ',' ORDER BY a::text) FROM unnest(datacl) a) FROM pg_database WHERE datname=current_database()",
                    &[],
                )
                .unwrap()
                .get(0)
        };
        let state = |f: &Fixture| {
            let others: Vec<String> = scratch_leftovers(f, &public)
                .into_iter()
                .filter(|x| !x.starts_with("database "))
                .collect();
            (acl(f), others)
        };
        let before = state(&f);
        assert!(before.0.is_some(), "{baseline}: {before:?}");
        f.sql("0001_connect.sql", refused);
        let r = f.command("plan");
        let d = diagnostic(&r, "DB_REPLAY_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
        assert_eq!(d["source"], "0001_connect.sql", "{d}");
        let message = d["message"].as_str().unwrap();
        assert!(
            message.contains("left the scratch role without the CONNECT privilege")
                && message.contains(&format!("it revoked CONNECT from {named}")),
            "{d}"
        );
        assert_eq!(r["scratch"]["cleanup"], "pass", "{r}");
        assert_eq!(state(&f), before);
        f.sql("0001_connect.sql", &kept);
        let r = f.command("plan");
        assert_eq!(r["outcome"], "pass", "{baseline}: {r}");
        assert_eq!(r["scratch"]["cleanup"], "pass", "{r}");
        assert_eq!(state(&f), before);
        // The scratch role can still connect, and the next plan passes.
        Client::connect(&f.scratch_url, NoTls).unwrap();
        assert_eq!(f.command("plan")["outcome"], "pass");
    }
}

/// #430: a replay that never cleaned up (its process died between
/// migrations) leaves committed migrations and NTNT's marker in scratch.
/// The next plan cleans them up, passes and says so. A scratch with the
/// marker and an object of another role, or with objects and no marker, is
/// refused as before and keeps everything.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn leftovers_of_an_unfinished_replay_are_cleaned_only_with_the_marker() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql(
        "0001_types.sql",
        "CREATE TYPE app.mood AS ENUM ('ok'); CREATE TABLE app.t (id int, m app.mood); CREATE EXTENSION citext WITH SCHEMA app;
         CREATE POLICY own ON app.t TO CURRENT_USER USING (true); GRANT CREATE ON SCHEMA public TO PUBLIC;",
    );
    f.sql(
        "0002_wait.sql",
        "SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    crash_during_replay(&f);
    let mut scratch = scratch_admin(&f);
    let left = |c: &mut Client| -> (bool, bool) {
        let r = c
            .query_one(
                "SELECT to_regnamespace('_ntnt_replay') IS NOT NULL, to_regclass('app.t') IS NOT NULL",
                &[],
            )
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert_eq!(left(&mut scratch), (true, true), "the crash left no marker");
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert!(r["scratch"]["recovered"]["replay_id"].is_string(), "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());

    // A marker and an object another role owns: refused, nothing dropped.
    crash_during_replay(&f);
    scratch
        .batch_execute("CREATE TABLE public.foreign_owned (id int)")
        .unwrap();
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_NOT_EMPTY").unwrap_or_else(|| panic!("{r}"));
    assert!(
        names(&d["objects"])
            .iter()
            .any(|o| o.contains("foreign_owned")),
        "{d}"
    );
    assert!(r["scratch"]["recovered"].is_null(), "{r}");
    assert_eq!(left(&mut scratch), (true, true));
    assert!(scratch
        .query_one(
            "SELECT to_regclass('public.foreign_owned') IS NOT NULL",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    scratch
        .batch_execute("DROP TABLE public.foreign_owned")
        .unwrap();
    assert_eq!(f.command("plan")["outcome"], "pass");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());

    // Objects without NTNT's marker are never dropped.
    let mut owner = Client::connect(&f.scratch_url, NoTls).unwrap();
    owner
        .batch_execute("CREATE SCHEMA leftover; CREATE TABLE leftover.t (id int)")
        .unwrap();
    let r = f.command("plan");
    assert!(has_code(&r, "DB_SCRATCH_NOT_EMPTY"), "{r}");
    assert!(r["scratch"]["recovered"].is_null(), "{r}");
    assert!(owner
        .query_one("SELECT to_regclass('leftover.t') IS NOT NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
}

/// A plan waiting for the replay lock, `pg_advisory_lock(1853124212, 430)`.
const REPLAY_LOCK_WAITER: &str = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND classid=1853124212 AND objid=430 AND objsubid=2 AND NOT granted)";

/// #430: a plan waits for the replay lock before it reads scratch, so two
/// replays of one scratch never interleave; past `migrator_wait_seconds`
/// it fails with DB_SCRATCH_BUSY. Scratch is untouched either way.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_plan_waits_for_another_replay_of_its_scratch() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    with_limits(&f, "migrator_wait_seconds = 300");
    let expected = f.command("plan");
    assert_eq!(expected["outcome"], "pass", "{expected}");
    let mut holder = scratch_admin(&f);
    holder
        .batch_execute("SELECT pg_advisory_lock(1853124212, 430)")
        .unwrap();
    let mut child = spawn_plan(&f);
    wait_for(
        &mut holder,
        REPLAY_LOCK_WAITER,
        &mut child,
        "plan waiting for the replay lock",
    );
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    holder
        .batch_execute("SELECT pg_advisory_unlock(1853124212, 430)")
        .unwrap();
    let r = report(child);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["plan_digest"], expected["plan_digest"]);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());

    let path = f.root.path().join("ntnt.toml");
    let manifest = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        manifest.replace("migrator_wait_seconds = 300", "migrator_wait_seconds = 1"),
    )
    .unwrap();
    holder
        .batch_execute("SELECT pg_advisory_lock(1853124212, 430)")
        .unwrap();
    let r = f.command("plan");
    assert!(has_code(&r, "DB_SCRATCH_BUSY"), "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430: two plans started together on one scratch both pass with the same
/// digest, and every migration of each replay ran holding the replay lock.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn concurrent_plans_on_one_scratch_are_serialized() {
    let f = Fixture::new();
    let public = public_state(&f);
    let holds_lock = "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_locks l WHERE l.locktype = 'advisory' AND l.classid = 1853124212 AND l.objid = 430 AND l.objsubid = 2 AND l.pid = pg_catalog.pg_backend_pid() AND l.granted) THEN RAISE EXCEPTION 'replay without the replay lock'; END IF; END $$;";
    f.sql(
        "0001_base.sql",
        &format!("{holds_lock} CREATE TABLE app.t (id int);"),
    );
    f.sql(
        "0002_more.sql",
        &format!("{holds_lock} ALTER TABLE app.t ADD COLUMN label text;"),
    );
    with_limits(&f, "migrator_wait_seconds = 300");
    let (a, b) = (spawn_plan(&f), spawn_plan(&f));
    let (a, b) = (report(a), report(b));
    assert_eq!(a["outcome"], "pass", "{a}");
    assert_eq!(b["outcome"], "pass", "{b}");
    assert_eq!(a["plan_digest"], b["plan_digest"]);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430: cleanup's DROP OWNED would revoke privileges granted to the
/// scratch role on objects it does not own, here the target database, so
/// such a scratch is refused before anything runs, and the grant stays.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_scratch_role_with_privileges_it_does_not_own_is_refused() {
    let mut f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    let grant = format!("GRANT CONNECT ON DATABASE {} TO {}", f.target, f.scratch);
    f.admin.batch_execute(&grant).unwrap();
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_UNSAFE").unwrap_or_else(|| panic!("{r}"));
    assert!(
        names(&d["objects"]).iter().any(|o| o.contains(&f.target)),
        "{d}"
    );
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    let held: bool = f
        .admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_database d CROSS JOIN LATERAL aclexplode(d.datacl) a JOIN pg_roles r ON r.oid=a.grantee WHERE d.datname=$2::text AND r.rolname=$1::text AND a.privilege_type='CONNECT')",
            &[&f.scratch, &f.target],
        )
        .unwrap()
        .get(0);
    assert!(held, "the grant to the scratch role was revoked");
    f.admin
        .batch_execute(&format!(
            "REVOKE CONNECT ON DATABASE {} FROM {}",
            f.target, f.scratch
        ))
        .unwrap();
    assert_eq!(f.command("plan")["outcome"], "pass");
}

/// #430: cleanup's DROP OWNED drops everything the scratch role owns, so a
/// plan refuses scratch-owned objects that the emptiness check does not see,
/// such as a large object and a publication, before it commits its marker,
/// and keeps them.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn objects_the_scratch_role_already_owns_are_refused_and_kept() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    let mut owner = Client::connect(&f.scratch_url, NoTls).unwrap();
    owner
        .batch_execute("SELECT pg_catalog.lo_create(4242); CREATE PUBLICATION ntnt_kept_pub;")
        .unwrap();
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_NOT_EMPTY").unwrap_or_else(|| panic!("{r}"));
    let objects = names(&d["objects"]);
    assert!(
        objects.iter().any(|o| o.contains("large object 4242")),
        "{d}"
    );
    assert!(objects.iter().any(|o| o.contains("ntnt_kept_pub")), "{d}");
    let kept = owner
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_largeobject_metadata WHERE oid = 4242) AND EXISTS (SELECT 1 FROM pg_publication WHERE pubname = 'ntnt_kept_pub') AND to_regnamespace('_ntnt_replay') IS NULL",
            &[],
        )
        .unwrap()
        .get::<_, bool>(0);
    assert!(
        kept,
        "the plan dropped what the scratch role owned or left a marker"
    );
    owner
        .batch_execute("SELECT pg_catalog.lo_unlink(4242); DROP PUBLICATION ntnt_kept_pub;")
        .unwrap();
    drop(owner);
    assert_eq!(f.command("plan")["outcome"], "pass");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430: a migration that releases NTNT's replay lock is refused before it
/// commits. While it still runs, another plan takes the lock, finds that
/// replay's marker and its session, and fails with DB_SCRATCH_BUSY instead
/// of cleaning up what the first replay still uses.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_migration_that_releases_the_replay_lock_is_refused() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    f.sql(
        "0002_unlock.sql",
        "SELECT pg_catalog.pg_advisory_unlock_all(); SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    let mut holder = scratch_admin(&f);
    holder
        .batch_execute("SELECT pg_advisory_lock(4242)")
        .unwrap();
    let mut first = spawn_plan(&f);
    let waiting = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=4242 AND objsubid=1 AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted)";
    wait_for(&mut holder, waiting, &mut first, "replay waiting in 0002");
    let second = f.command("plan");
    let d = diagnostic(&second, "DB_SCRATCH_BUSY").unwrap_or_else(|| panic!("{second}"));
    assert!(
        d["message"].as_str().unwrap().contains("still connected"),
        "{d}"
    );
    assert!(second["scratch"]["recovered"].is_null(), "{second}");
    let kept: bool = holder
        .query_one(
            "SELECT to_regclass('app.t') IS NOT NULL AND to_regnamespace('_ntnt_replay') IS NOT NULL",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(kept, "the second plan cleaned up a running replay");
    holder
        .batch_execute("SELECT pg_advisory_unlock(4242)")
        .unwrap();
    let r = report(first);
    let d = diagnostic(&r, "DB_REPLAY_UNSUPPORTED").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(d["source"], "0002_unlock.sql", "{d}");
    assert!(
        d["message"].as_str().unwrap().contains("replay lock"),
        "{d}"
    );
    assert_eq!(r["scratch"]["cleanup"], "pass", "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    std::fs::remove_file(f.root.path().join("migrations/0002_unlock.sql")).unwrap();
    assert_eq!(f.command("plan")["outcome"], "pass");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// The scratch database's privileges (as sets), connection limit, comment
/// and settings, and the scratch role's settings, in `f`'s scratch.
fn database_state(f: &Fixture) -> String {
    scratch_admin(f)
        .query_one(
            "SELECT coalesce((SELECT string_agg(e, ',' ORDER BY e) FROM (SELECT a.grantor::regrole||'>'||a.grantee||':'||a.privilege_type||':'||a.is_grantable AS e FROM aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a) x), '-')
              ||' | '||d.datconnlimit||' | '||coalesce(shobj_description(d.oid, 'pg_database'), '-')
              ||' | '||coalesce((SELECT string_agg(s.setdatabase||'/'||s.setrole||'='||array_to_string(s.setconfig, ';'), ',' ORDER BY s.setdatabase, s.setrole) FROM pg_db_role_setting s WHERE s.setdatabase=d.oid OR s.setrole=d.datdba), '-')
             FROM pg_database d WHERE d.datname=current_database()",
            &[],
        )
        .unwrap()
        .get(0)
}

/// #430 review: histories #456 accepted change the scratch database and the
/// scratch role in ways cleanup can undo, so they still plan: settings
/// (also a list the database already had, written back element by
/// element), privileges, a connection limit and a comment of the database,
/// settings of the role, and `public` dropped and created again for
/// `pg_database_owner`. Cleanup restores each, and the next plan gets the
/// same digest.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn scratch_database_and_role_settings_are_restored_after_replay() {
    let mut f = Fixture::new();
    f.admin
        .batch_execute(&format!(
            "ALTER DATABASE {0} SET search_path TO \"$user\", public, \"odd, name\"; COMMENT ON DATABASE {0} IS 'scratch'",
            f.scratch
        ))
        .unwrap();
    let public = public_state(&f);
    let (left, state) = (scratch_leftovers(&f, &public), database_state(&f));
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int PRIMARY KEY);");
    let on_db =
        |what: &str| format!("DO $$ BEGIN EXECUTE format('{what}', current_database()); END $$;");
    for sql in [
        on_db("ALTER DATABASE %I SET search_path TO app, public"),
        on_db("GRANT CONNECT ON DATABASE %I TO PUBLIC"),
        on_db("REVOKE CONNECT ON DATABASE %I FROM PUBLIC"),
        on_db(&format!("GRANT CONNECT ON DATABASE %I TO {}", f.target)),
        "DROP SCHEMA public; CREATE SCHEMA public AUTHORIZATION pg_database_owner; GRANT USAGE ON SCHEMA public TO PUBLIC;".into(),
        on_db("ALTER DATABASE %I RESET ALL"),
        on_db("ALTER DATABASE %I SET timezone = ''Europe/Paris''"),
        on_db("ALTER DATABASE %I CONNECTION LIMIT 50"),
        on_db("REVOKE TEMPORARY ON DATABASE %I FROM PUBLIC"),
        on_db("COMMENT ON DATABASE %I IS ''replayed''"),
        "DO $$ BEGIN EXECUTE 'ALTER ROLE CURRENT_USER SET work_mem = ''8MB'''; END $$;".into(),
        on_db("ALTER ROLE CURRENT_USER IN DATABASE %I SET work_mem = ''8MB''"),
    ] {
        f.sql("0002_case.sql", &sql);
        let r = f.command("plan");
        assert_eq!(r["outcome"], "pass", "{sql}: {r}");
        assert_eq!(r["scratch"]["cleanup"], "pass", "{sql}: {r}");
        assert_eq!(scratch_leftovers(&f, &public), left, "{sql}: {r}");
        assert_eq!(database_state(&f), state, "{sql}: {r}");
        assert_eq!(f.command("plan")["plan_digest"], r["plan_digest"], "{sql}");
    }
}

/// Run `sql` as `f`'s scratch role in scratch, then wait until that session
/// has ended: a plan takes another connected session of the scratch role
/// for an unfinished replay that may still be running.
fn as_scratch_role(f: &Fixture, sql: &str) {
    let mut owner = Client::connect(&f.scratch_url, NoTls).unwrap();
    owner.batch_execute(sql).unwrap();
    drop(owner);
    let mut admin = scratch_admin(f);
    let start = std::time::Instant::now();
    while admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename=$1::text)",
            &[&f.scratch],
        )
        .unwrap()
        .get::<_, bool>(0)
    {
        assert!(start.elapsed() < LOCK_WAIT_DEADLINE, "the session did not end");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Run `command` on a manifest whose target is `f`'s scratch database (as
/// another project could be configured) and whose scratch is `f`'s target.
fn command_on_scratch_as_target(f: &Fixture, command: &str) -> Value {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("migrations")).unwrap();
    std::fs::write(
        root.path().join("ntnt.toml"),
        format!(
            "[databases.app]\nengine = \"postgresql\"\nmigrations_dir = \"migrations\"\nschema = \"b\"\napproval_secret = \"DB_APPROVAL\"\n[databases.app.targets.test]\nconnection_secret = \"DB_TARGET\"\ndatabase = \"{0}\"\nrole = \"{0}\"\n[databases.app.scratch]\nconnection_secret = \"DB_SCRATCH\"\ndatabase = \"{1}\"\nrole = \"{1}\"\ndisposable = true\n[secrets.DB_TARGET]\n[secrets.DB_SCRATCH]\n[secrets.DB_APPROVAL]\n",
            f.scratch, f.target
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", command, "--target", "test", "--json"])
        .env("DB_TARGET", &f.scratch_url)
        .env("DB_SCRATCH", &f.target_url)
        .env_remove("DB_APPROVAL")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(root.path())
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

/// #430 review (P1): a scratch database that holds an unfinished replay's
/// marker but also what NTNT cannot prove that replay committed is refused
/// and keeps everything: another project's ledger (`_ntnt`, named as such),
/// objects written outside the replay's transactions, and a marker copied
/// from another database or cluster. Such a database used as a target is
/// refused with DB_TARGET_IS_SCRATCH. Once only the replay's leftovers are
/// there, also what an exception block (a subtransaction) created, the next
/// plan cleans them up.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn recovery_never_drops_what_it_cannot_prove_the_replay_committed() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql(
        "0001_types.sql",
        "CREATE TYPE app.mood AS ENUM ('ok'); CREATE TABLE app.t (id int, m app.mood);
         DO $$ BEGIN BEGIN CREATE TABLE app.in_block (id int); EXCEPTION WHEN duplicate_table THEN NULL; END; END $$;",
    );
    f.sql(
        "0002_wait.sql",
        "SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    crash_during_replay(&f);
    // Another project, configured with this database as its target.
    let r = command_on_scratch_as_target(&f, "status");
    let d = diagnostic(&r, "DB_TARGET_IS_SCRATCH").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(names(&d["objects"]), ["schema:_ntnt_replay"], "{d}");
    // That project migrated here with an earlier NTNT: its ledger and data.
    as_scratch_role(
        &f,
        "CREATE SCHEMA _ntnt; CREATE TABLE _ntnt.schema_migrations_v2 (position int); INSERT INTO _ntnt.schema_migrations_v2 VALUES (1);
         CREATE SCHEMA b; CREATE TABLE b.accounts (id int PRIMARY KEY, name text); INSERT INTO b.accounts VALUES (1, 'kept'), (2, 'kept');",
    );
    let mut owner = scratch_admin(&f);
    let kept = |c: &mut Client| -> (i64, bool) {
        let r = c
            .query_one(
                "SELECT (SELECT count(*) FROM b.accounts), to_regnamespace('_ntnt_replay') IS NOT NULL AND to_regclass('app.t') IS NOT NULL",
                &[],
            )
            .unwrap();
        (r.get(0), r.get(1))
    };
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_NOT_EMPTY").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(names(&d["objects"]), ["schema:_ntnt"], "{d}");
    assert!(
        d["message"].as_str().unwrap().contains("migration ledger"),
        "{d}"
    );
    assert!(r["scratch"]["recovered"].is_null(), "{r}");
    assert_eq!(r["scratch"]["cleanup"], "not_needed", "{r}");
    assert_eq!(kept(&mut owner), (2, true));
    assert!(owner
        .query_one(
            "SELECT to_regclass('_ntnt.schema_migrations_v2') IS NOT NULL",
            &[]
        )
        .unwrap()
        .get::<_, bool>(0));
    // Without the ledger, what was written outside the replay is named.
    as_scratch_role(&f, "DROP SCHEMA _ntnt CASCADE");
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_NOT_EMPTY").unwrap_or_else(|| panic!("{r}"));
    let objects = names(&d["objects"]);
    assert!(objects.contains(&"schema b"), "{d}");
    assert!(objects.contains(&"table b.accounts"), "{d}");
    assert!(!objects.iter().any(|o| o.contains("app.")), "{d}");
    assert_eq!(kept(&mut owner), (2, true));
    as_scratch_role(&f, "DROP SCHEMA b CASCADE");
    // A marker that names another database or cluster.
    let mut admin = scratch_admin(&f);
    for (column, tamper, cast) in [
        (
            "database_oid",
            "UPDATE _ntnt_replay.replay_v1 SET database_oid = database_oid::int8 + 1",
            "oid",
        ),
        (
            "system_identifier",
            "UPDATE _ntnt_replay.replay_v1 SET system_identifier = system_identifier || '0'",
            "text",
        ),
    ] {
        let original: String = admin
            .query_one(
                &format!("SELECT {column}::text FROM _ntnt_replay.replay_v1"),
                &[],
            )
            .unwrap()
            .get(0);
        admin.batch_execute(tamper).unwrap();
        let r = f.command("plan");
        let d = diagnostic(&r, "DB_SCRATCH_NOT_EMPTY").unwrap_or_else(|| panic!("{r}"));
        assert!(
            d["message"].as_str().unwrap().contains("copied here"),
            "{d}"
        );
        let left: bool = owner
            .query_one(
                "SELECT to_regnamespace('_ntnt_replay') IS NOT NULL AND to_regclass('app.t') IS NOT NULL",
                &[],
            )
            .unwrap()
            .get(0);
        assert!(left, "{column}: the leftovers were dropped");
        admin
            .execute(
                &format!("UPDATE _ntnt_replay.replay_v1 SET {column} = $1::text::{cast}"),
                &[&original],
            )
            .unwrap();
    }
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert!(r["scratch"]["recovered"]["replay_id"].is_string(), "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    let r = command_on_scratch_as_target(&f, "status");
    assert!(diagnostic(&r, "DB_TARGET_IS_SCRATCH").is_none(), "{r}");
}

/// #430 review: cleanup drops nothing without the replay lock. A migration
/// released it and another session took it, so cleanup fails with
/// DB_SCRATCH_CLEANUP and keeps what the replay committed; the next plan,
/// holding the lock again, recovers it.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn cleanup_without_the_replay_lock_drops_nothing_and_the_next_plan_recovers() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    f.sql(
        "0002_unlock.sql",
        "SELECT pg_catalog.pg_advisory_unlock_all(); SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    let mut holder = scratch_admin(&f);
    holder
        .batch_execute("SELECT pg_advisory_lock(4242)")
        .unwrap();
    let mut child = spawn_plan(&f);
    let waiting = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=4242 AND objsubid=1 AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted)";
    wait_for(&mut holder, waiting, &mut child, "replay waiting in 0002");
    holder
        .batch_execute("SELECT pg_advisory_lock(1853124212, 430); SELECT pg_advisory_unlock(4242)")
        .unwrap();
    let r = report(child);
    assert!(diagnostic(&r, "DB_REPLAY_UNSUPPORTED").is_some(), "{r}");
    let d = diagnostic(&r, "DB_SCRATCH_CLEANUP").unwrap_or_else(|| panic!("{r}"));
    assert!(
        d["message"].as_str().unwrap().contains("replay lock"),
        "{d}"
    );
    assert_eq!(r["scratch"]["cleanup"], "unknown", "{r}");
    let kept: bool = holder
        .query_one(
            "SELECT to_regclass('app.t') IS NOT NULL AND to_regnamespace('_ntnt_replay') IS NOT NULL",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(kept, "cleanup dropped without the replay lock");
    holder
        .batch_execute("SELECT pg_advisory_unlock(1853124212, 430)")
        .unwrap();
    std::fs::remove_file(f.root.path().join("migrations/0002_unlock.sql")).unwrap();
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert!(r["scratch"]["recovered"]["replay_id"].is_string(), "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430 review: a marker the replay committed that another session drops
/// while the replay runs fails cleanup with DB_SCRATCH_CLEANUP, so the
/// results are discarded, and nothing is dropped without it.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_committed_marker_that_disappears_fails_cleanup() {
    let f = Fixture::new();
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    f.sql(
        "0002_wait.sql",
        "SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    let mut holder = scratch_admin(&f);
    holder
        .batch_execute("SELECT pg_advisory_lock(4242)")
        .unwrap();
    let mut child = spawn_plan(&f);
    let waiting = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=4242 AND objsubid=1 AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted)";
    wait_for(&mut holder, waiting, &mut child, "replay waiting in 0002");
    holder
        .batch_execute("DROP SCHEMA _ntnt_replay CASCADE; SELECT pg_advisory_unlock(4242)")
        .unwrap();
    let r = report(child);
    let d = diagnostic(&r, "DB_SCRATCH_CLEANUP").unwrap_or_else(|| panic!("{r}"));
    assert!(d["message"].as_str().unwrap().contains("is gone"), "{d}");
    assert_eq!(r["scratch"]["cleanup"], "unknown", "{r}");
    assert_ne!(r["outcome"], "pass", "{r}");
    assert!(holder
        .query_one("SELECT to_regclass('app.t') IS NOT NULL", &[])
        .unwrap()
        .get::<_, bool>(0));
}

/// The plan's report; a plan still running after LOCK_WAIT_DEADLINE is
/// killed and fails the test (its output is read meanwhile, so a full pipe
/// cannot hold it up).
fn report_within(child: std::process::Child, what: &str) -> Value {
    let id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(LOCK_WAIT_DEADLINE) {
        Ok(output) => {
            let output = output.unwrap();
            serde_json::from_slice(&output.stdout)
                .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
        }
        Err(_) => {
            let _ = std::process::Command::new("kill")
                .args(["-9", &id.to_string()])
                .status();
            panic!("{what}: the plan did not return within {LOCK_WAIT_DEADLINE:?}");
        }
    }
}

/// #430 round 2: cleanup waits at most 2 seconds for each of its locks. A
/// session that holds a lock on a table the replay committed, until the plan
/// has returned, makes cleanup fail with DB_SCRATCH_CLEANUP naming that
/// session, and keep what the replay committed and the marker; a cleanup
/// that waited for the lock would never return before the hang guard. Once
/// the lock is gone, the next plan recovers.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_lock_on_a_replayed_table_fails_cleanup_fast_and_the_next_plan_recovers() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    f.sql(
        "0002_wait.sql",
        "SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    let mut holder = scratch_admin(&f);
    holder
        .batch_execute("SELECT pg_advisory_lock(4242)")
        .unwrap();
    let mut child = spawn_plan(&f);
    let waiting = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=4242 AND objsubid=1 AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted)";
    wait_for(&mut holder, waiting, &mut child, "replay waiting in 0002");
    let mut blocker = scratch_admin(&f);
    blocker
        .batch_execute("BEGIN; LOCK TABLE app.t IN ACCESS SHARE MODE")
        .unwrap();
    let row = blocker
        .query_one("SELECT pg_backend_pid(), current_user::text", &[])
        .unwrap();
    let (pid, role): (i32, String) = (row.get(0), row.get(1));
    holder
        .batch_execute("SELECT pg_advisory_unlock(4242)")
        .unwrap();
    let r = report_within(child, "cleanup blocked by a lock on app.t");
    let d = diagnostic(&r, "DB_SCRATCH_CLEANUP").unwrap_or_else(|| panic!("{r}"));
    let message = d["message"].as_str().unwrap();
    assert!(message.contains("55P03"), "{d}");
    assert!(message.contains(&format!("pid {pid} (role {role})")), "{d}");
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("rerun plan, status or verify"),
        "{d}"
    );
    assert_eq!(r["scratch"]["cleanup"], "unknown", "{r}");
    let kept: bool = holder
        .query_one(
            "SELECT to_regclass('app.t') IS NOT NULL AND to_regnamespace('_ntnt_replay') IS NOT NULL",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(
        kept,
        "cleanup dropped what the replay committed, or its marker"
    );
    blocker.batch_execute("ROLLBACK").unwrap();
    std::fs::remove_file(f.root.path().join("migrations/0002_wait.sql")).unwrap();
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert!(r["scratch"]["recovered"]["replay_id"].is_string(), "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430 review: replay has its own deadline inside inspection, so a plan
/// whose replay runs past it is cut short (its statement cancelled) with
/// DB_TIMEOUT, and cleanup still runs and empties scratch. Each migration
/// stays within the statement limit; together they pass the default
/// deadline, so this takes about two minutes.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_replay_cut_short_by_its_deadline_is_cleaned_up() {
    let f = Fixture::new();
    let public = public_state(&f);
    f.sql("0001_base.sql", "CREATE TABLE app.t (id int);");
    f.sql("0002_wait.sql", "SELECT pg_catalog.pg_sleep(70);");
    f.sql("0003_wait.sql", "SELECT pg_catalog.pg_sleep(70);");
    let started = std::time::Instant::now();
    let r = f.command("plan");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(140),
        "{:?}",
        started.elapsed()
    );
    let d = diagnostic(&r, "DB_TIMEOUT").unwrap_or_else(|| panic!("{r}"));
    assert!(
        d["message"].as_str().unwrap().contains("Scratch replay"),
        "{d}"
    );
    assert_eq!(r["scratch"]["cleanup"], "pass", "{r}");
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}

/// #430 review (Greptile P1): a replay killed after a migration committed a
/// timezone for the scratch database leaves new sessions with it, which no
/// longer match the target. The next plan cleans the leftovers up first,
/// restoring the setting, and reports the mismatch; the one after passes.
#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI db catalog lane"]
fn a_setting_an_unfinished_replay_left_is_restored_before_the_mismatch() {
    let f = Fixture::new();
    let public = public_state(&f);
    let state = database_state(&f);
    f.sql(
        "0001_timezone.sql",
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET timezone = ''Pacific/Chatham''', current_database()); END $$;
         CREATE TABLE app.t (id int);",
    );
    f.sql(
        "0002_wait.sql",
        "SET LOCAL lock_timeout = 0; SELECT pg_catalog.pg_advisory_xact_lock(4242);",
    );
    crash_during_replay(&f);
    assert_ne!(database_state(&f), state, "the crash left no setting");
    let r = f.command("plan");
    let d = diagnostic(&r, "DB_SCRATCH_MISMATCH").unwrap_or_else(|| panic!("{r}"));
    assert!(d["message"].as_str().unwrap().contains("rerun"), "{d}");
    assert_eq!(database_state(&f), state);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
    std::fs::remove_file(f.root.path().join("migrations/0002_wait.sql")).unwrap();
    let r = f.command("plan");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(database_state(&f), state);
    assert_eq!(scratch_leftovers(&f, &public), Vec::<String>::new());
}
