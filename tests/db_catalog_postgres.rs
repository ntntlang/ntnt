//! Real PostgreSQL contracts. Ignored locally unless explicitly selected; CI runs all
//! with a dedicated admin URL. Never use production credentials for this fixture.
use postgres::{Client, NoTls};
use serde_json::Value;
use std::process::Command;

struct Fixture {
    admin: Client,
    root: tempfile::TempDir,
    target: String,
    scratch: String,
    target_url: String,
    scratch_url: String,
}
impl Fixture {
    fn new() -> Self {
        let url = std::env::var("NTNT_DB_TEST_ADMIN")
            .expect("explicit disposable PostgreSQL fixture URL required");
        let mut admin = Client::connect(&url, NoTls).unwrap();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let target = format!("ntnt245_t_{token}");
        let scratch = format!("ntnt245_s_{token}");
        for name in [&target, &scratch] {
            admin.batch_execute(&format!("CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD 'ntnt245_fixture_only'")).unwrap();
            admin
                .batch_execute(&format!(
                    "CREATE DATABASE {name} OWNER {name} TEMPLATE template0"
                ))
                .unwrap();
        }
        // URL supplied by fixture uses libpq key/value form, with trust only in
        // this isolated fixture. Production secrets never enter a process arg.
        let endpoint = std::env::var("NTNT_DB_TEST_HOST").expect("fixture host required");
        let target_url = format!("host={endpoint} user={target} dbname={target} password=ntnt245_fixture_only sslmode=disable");
        let scratch_url =
            format!("host={endpoint} user={scratch} dbname={scratch} password=ntnt245_fixture_only sslmode=disable");
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("migrations")).unwrap();
        std::fs::write(
            root.path().join("ntnt.toml"),
            format!(
                r#"
[database]
engine = "postgresql"
migrations_dir = "migrations"
schema = "app"
[database.targets.test]
connection_secret = "DB_TARGET"
database = "{target}"
role = "{target}"
[database.scratch]
connection_secret = "DB_SCRATCH"
database = "{scratch}"
role = "{scratch}"
disposable = true
[secrets.DB_TARGET]
[secrets.DB_SCRATCH]
"#
            ),
        )
        .unwrap();
        let mut conn = Client::connect(&target_url, NoTls).unwrap();
        conn.batch_execute("CREATE SCHEMA app").unwrap();
        Self {
            admin,
            root,
            target,
            scratch,
            target_url,
            scratch_url,
        }
    }
    fn command(&self, command: &str) -> Value {
        self.command_args(command, &[])
    }
    fn command_args(&self, command: &str, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args(["db", command, "--target", "test", "--json"])
            .args(args)
            .env("DB_TARGET", &self.target_url)
            .env("DB_SCRATCH", &self.scratch_url)
            .env("NTNT_SECRETS_PROVIDER", "env")
            .env("NTNT_ENV", "development")
            .current_dir(self.root.path())
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    }
    fn seed_first(&self, sql: &str) -> Value {
        self.sql("0001_initial.sql", sql);
        let plan = self.command("plan");
        assert_eq!(plan["plan"]["candidate"]["outcome"], "pass", "{plan}");
        let checksum = plan["source"]["migrations"][0]["checksum"]
            .as_str()
            .unwrap();
        let mut c = self.target();
        c.batch_execute(sql).unwrap();
        c.batch_execute("CREATE SCHEMA _ntnt; CREATE TABLE _ntnt.schema_migrations_v1 (position bigint NOT NULL PRIMARY KEY,id text NOT NULL,name text NOT NULL,checksum text NOT NULL,state text NOT NULL)").unwrap();
        c.execute(
            "INSERT INTO _ntnt.schema_migrations_v1 VALUES(1,'1','0001_initial.sql',$1,'applied')",
            &[&checksum],
        )
        .unwrap();
        plan
    }
    fn target(&self) -> Client {
        Client::connect(&self.target_url, NoTls).unwrap()
    }
    fn sql(&self, name: &str, sql: &str) {
        std::fs::write(self.root.path().join("migrations").join(name), sql).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [&self.target, &self.scratch] {
            let _ = self
                .admin
                .batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"));
            let _ = self.admin.batch_execute(&format!("DROP ROLE {name}"));
        }
    }
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
    for sql in [
        "SELECT pg_advisory_lock(245)",
        "CREATE TABLE app.bad(id int DEFAULT pg_sleep(30))",
        "DO $$ BEGIN RAISE EXCEPTION 'secret_payload'; END $$;",
        "CREATE INDEX CONCURRENTLY bad ON app.t(id)",
    ] {
        f.sql("0002_bad.sql", sql);
        let r = f.command("plan");
        assert!(
            has_code(&r, "DB_REPLAY_UNSUPPORTED"),
            "{}",
            r["diagnostics"]
        );
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
        .batch_execute("CREATE VIEW app.slow AS SELECT 1 AS sleeper FROM pg_sleep(30)")
        .unwrap();
    let r = f.command("verify");
    assert_eq!(r["drift"]["outcome"], "unknown");
    assert!(has_code(&r, "DB_COVERAGE_INCOMPLETE"));
    assert!(r["catalog"]["uncovered"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("view:slow")));
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
        if start.elapsed() > std::time::Duration::from_millis(1500) {
            let _ = child.kill();
            panic!("inspector did not reach the history lock");
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
