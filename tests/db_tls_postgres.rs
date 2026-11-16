//! `ntnt db` over TLS against a disposable TLS-enabled PostgreSQL started by
//! `scripts/ci/postgres-tls-fixture.sh`. Ignored by default; mandatory in the
//! CI db-catalog lane on PostgreSQL 16/17/18.
//!
//! The fixture's server certificate is issued by a throwaway CA for IP
//! 127.0.0.1 only, so `host=127.0.0.1` matches and any other host name does not.
use postgres::{Client, NoTls};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::process::Command;

const PASSWORD: &str = "ntnt344_fixture_only";

fn env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is required: start scripts/ci/postgres-tls-fixture.sh"))
}

struct TlsFixture {
    admin: Client,
    root: tempfile::TempDir,
    target: String,
    scratch: String,
    port: String,
    ca: String,
    other_ca: String,
}
impl TlsFixture {
    fn new() -> Self {
        let mut admin = Client::connect(&env("NTNT_DB_TLS_TEST_ADMIN"), NoTls).unwrap();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let target = format!("ntnt344_t_{token}");
        let scratch = format!("ntnt344_s_{token}");
        for name in [&target, &scratch] {
            admin.batch_execute(&format!("CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{PASSWORD}'")).unwrap();
            admin
                .batch_execute(&format!(
                    "CREATE DATABASE {name} OWNER {name} TEMPLATE template0"
                ))
                .unwrap();
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("migrations")).unwrap();
        std::fs::write(
            root.path().join("ntnt.toml"),
            format!(
                r#"
[databases.app]
engine = "postgresql"
migrations_dir = "migrations"
schema = "public"
approval_secret = "DB_APPROVAL"
[databases.app.targets.test]
connection_secret = "DB_TARGET"
database = "{target}"
role = "{target}"
[databases.app.scratch]
connection_secret = "DB_SCRATCH"
database = "{scratch}"
role = "{scratch}"
disposable = true
[secrets.DB_TARGET]
[secrets.DB_SCRATCH]
[secrets.DB_APPROVAL]
"#
            ),
        )
        .unwrap();
        std::fs::write(
            root.path().join("migrations/0001_accounts.sql"),
            "CREATE TABLE public.accounts (id bigint PRIMARY KEY, email text NOT NULL);",
        )
        .unwrap();
        Self {
            admin,
            root,
            target,
            scratch,
            port: env("NTNT_DB_TLS_TEST_PORT"),
            ca: env("NTNT_DB_TLS_TEST_CA"),
            other_ca: env("NTNT_DB_TLS_TEST_OTHER_CA"),
        }
    }
    /// libpq key/value secret for `role`; `address` is always 127.0.0.1, `host`
    /// is the name TLS verifies.
    fn secret(&self, role: &str, host: &str, tls: &str) -> String {
        format!(
            "host={host} hostaddr=127.0.0.1 port={} user={role} dbname={role} password={PASSWORD} {tls}",
            self.port
        )
    }
    fn process(&self, command: &str, host: &str, tls: &str) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        c.args(["db", command, "--target", "test"])
            .env("DB_TARGET", self.secret(&self.target, host, tls))
            .env("DB_SCRATCH", self.secret(&self.scratch, host, tls))
            .env_remove("DB_APPROVAL")
            // Default trust is the host CA store only.
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .env("NTNT_ENV", "development")
            .current_dir(self.root.path());
        c
    }
    fn json(&self, mut c: Command) -> Value {
        let output = c.arg("--json").output().unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    }
    fn run(&self, command: &str, host: &str, tls: &str) -> Value {
        self.json(self.process(command, host, tls))
    }
    fn table_exists(&self) -> bool {
        let mut c = Client::connect(
            &format!(
                "host=127.0.0.1 port={} user={} dbname={} password={PASSWORD} sslmode=disable",
                self.port, self.target, self.target
            ),
            NoTls,
        )
        .unwrap();
        c.query_one("SELECT to_regclass('public.accounts') IS NOT NULL", &[])
            .unwrap()
            .get(0)
    }
}
impl Drop for TlsFixture {
    fn drop(&mut self) {
        let _ = self
            .admin
            .batch_execute("SET lock_timeout = '10s'; SET statement_timeout = '30s'");
        for name in [&self.target, &self.scratch] {
            let _ = self
                .admin
                .batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"));
            let _ = self.admin.batch_execute(&format!("DROP ROLE {name}"));
        }
    }
}

/// A connection that cannot be established leaves the observation `unknown`
/// (nothing was inspected), with a specific DB_TLS diagnostic.
fn tls_failure(r: &Value) -> String {
    assert_eq!(r["outcome"], "unknown", "{r}");
    let d = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_TLS")
        .unwrap_or_else(|| panic!("expected DB_TLS: {r}"));
    d["message"].as_str().unwrap().to_string()
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn verify_full_runs_the_whole_workflow_and_plans_bind_the_transport() {
    let f = TlsFixture::new();
    let tls = format!("sslmode=verify-full sslrootcert={}", f.ca);
    let status = f.run("status", "127.0.0.1", &tls);
    assert_eq!(status["outcome"], "pass", "{status}");
    let transport = &status["target"]["identity"]["transport"];
    assert_eq!(transport["sslmode"], "verify-full", "{transport}");
    assert_eq!(transport["roots"], "file");
    assert_eq!(
        transport["roots_sha256"],
        hex::encode(Sha256::digest(std::fs::read(&f.ca).unwrap()))
    );
    // The server, not only the client, reports this session as encrypted.
    assert_eq!(transport["encrypted"], true, "{transport}");
    assert!(transport["tls_version"]
        .as_str()
        .is_some_and(|v| v.starts_with("TLSv1.")));
    assert_eq!(
        status["scratch"]["identity"]["transport"]["encrypted"],
        true
    );

    let mut plan = f.process("plan", "127.0.0.1", &tls);
    plan.args(["--out", "review.json"]);
    let plan = f.json(plan);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(
        plan["plan"]["binding"]["target"]["transport"]["sslmode"],
        "verify-full"
    );
    let digest = plan["plan_digest"].as_str().unwrap().to_string();

    // The reviewed plan cannot run over a weaker transport.
    let mut weaker = f.process("migrate", "127.0.0.1", "sslmode=require");
    weaker
        .args(["--plan", "review.json"])
        .env("DB_APPROVAL", &digest);
    let weaker = f.json(weaker);
    assert_eq!(weaker["outcome"], "fail", "{weaker}");
    assert!(
        weaker["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "DB_PLAN_STALE"),
        "{weaker}"
    );
    assert!(!f.table_exists());

    let mut migrate = f.process("migrate", "127.0.0.1", &tls);
    migrate
        .args(["--plan", "review.json"])
        .env("DB_APPROVAL", &digest);
    let applied = f.json(migrate);
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert!(f.table_exists());
    let verify = f.run("verify", "127.0.0.1", &tls);
    assert_eq!(verify["outcome"], "pass", "{verify}");
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn certificate_and_host_name_failures_stop_before_any_sql() {
    let f = TlsFixture::new();
    // The certificate is for 127.0.0.1, not this name; verify-full refuses it.
    let message = tls_failure(&f.run(
        "status",
        "db.ntnt-tls.invalid",
        &format!("sslmode=verify-full sslrootcert={}", f.ca),
    ));
    assert!(message.contains("host name"), "{message}");
    // verify-ca checks the chain only, so the same name is accepted.
    let chain_only = f.run(
        "status",
        "db.ntnt-tls.invalid",
        &format!("sslmode=verify-ca sslrootcert={}", f.ca),
    );
    assert_eq!(chain_only["outcome"], "pass", "{chain_only}");
    assert_eq!(
        chain_only["target"]["identity"]["transport"]["sslmode"],
        "verify-ca"
    );
    // Neither an unrelated CA nor the host CA store (the default) trusts the
    // fixture CA.
    for tls in [
        format!("sslmode=verify-full sslrootcert={}", f.other_ca),
        "sslmode=verify-full".to_string(),
        "sslmode=verify-ca sslrootcert=system".to_string(),
    ] {
        let message = tls_failure(&f.run("status", "127.0.0.1", &tls));
        assert!(message.contains("trusted CA"), "{tls}: {message}");
    }
    // Nothing ran: no plan was possible, so nothing could be migrated.
    assert!(!f.table_exists());
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn require_is_encrypted_but_reported_as_unverified() {
    let f = TlsFixture::new();
    let status = f.run("status", "127.0.0.1", "sslmode=require");
    assert_eq!(status["outcome"], "pass", "{status}");
    let transport = &status["target"]["identity"]["transport"];
    assert_eq!(transport["sslmode"], "require");
    assert_eq!(transport["roots"], Value::Null);
    assert_eq!(transport["encrypted"], true);
    let human = f
        .process("status", "127.0.0.1", "sslmode=require")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(
        stdout.contains("Transport: TLS without server verification (sslmode=require)"),
        "{stdout}"
    );
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn url_secrets_carry_tls_parameters() {
    let f = TlsFixture::new();
    let url = |role: &str| {
        format!(
            "postgresql://{role}:{PASSWORD}@127.0.0.1:{}/{role}?application_name=ignored&sslmode=verify-full&sslrootcert={}",
            f.port, f.ca
        )
    };
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "status", "--target", "test", "--json"])
        .env("DB_TARGET", url(&f.target))
        .env("DB_SCRATCH", url(&f.scratch))
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env("NTNT_ENV", "development")
        .current_dir(f.root.path())
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(status["outcome"], "pass", "{status}");
    assert_eq!(
        status["target"]["identity"]["transport"]["sslmode"],
        "verify-full"
    );
    assert_eq!(status["target"]["identity"]["transport"]["encrypted"], true);
}
