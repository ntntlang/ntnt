//! `ntnt db` over TLS against a disposable TLS-enabled PostgreSQL started by
//! `scripts/ci/postgres-tls-fixture.sh`. Ignored by default; mandatory in the
//! CI db-catalog lane on PostgreSQL 16/17/18.
//!
//! The fixture's server certificate is issued by a throwaway CA for IP
//! 127.0.0.1 only, so `host=127.0.0.1` matches and any other host name does not.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const PASSWORD: &str = "ntnt344_fixture_only";
/// Hang guard for every fixture database operation (connect, authenticate,
/// query): a stalled server fails the test instead of hanging it. Only a hang
/// reaches it; CREATE/DROP DATABASE on a busy server stays far below it.
const HANG_GUARD: Duration = Duration::from_secs(120);

fn env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is required: start scripts/ci/postgres-tls-fixture.sh"))
}

/// Run one plaintext statement batch, or a boolean query, against the fixture
/// with a whole-operation deadline.
fn fixture_sql(url: &str, sql: &str, query: bool) -> Option<bool> {
    let url = url.to_string();
    let sql = sql.to_string();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            tokio::time::timeout(HANG_GUARD, async move {
                let (client, connection) = url
                    .parse::<tokio_postgres::Config>()
                    .unwrap()
                    .connect(tokio_postgres::NoTls)
                    .await
                    .unwrap();
                let task = tokio::spawn(connection);
                let result = if query {
                    Some(client.query_one(&sql, &[]).await.unwrap().get::<_, bool>(0))
                } else {
                    client.batch_execute(&sql).await.unwrap();
                    None
                };
                drop(client);
                let _ = task.await;
                result
            })
            .await
            .unwrap_or_else(|_| panic!("fixture PostgreSQL did not answer within {HANG_GUARD:?}"))
        })
}

struct TlsFixture {
    admin: String,
    root: tempfile::TempDir,
    target: String,
    scratch: String,
    port: String,
    ca: String,
    other_ca: String,
}
impl TlsFixture {
    fn new() -> Self {
        let admin = env("NTNT_DB_TLS_TEST_ADMIN");
        let token = uuid::Uuid::new_v4().simple().to_string();
        let target = format!("ntnt344_t_{token}");
        let scratch = format!("ntnt344_s_{token}");
        for name in [&target, &scratch] {
            fixture_sql(&admin, &format!("CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{PASSWORD}'"), false);
            fixture_sql(
                &admin,
                &format!("CREATE DATABASE {name} OWNER {name} TEMPLATE template0"),
                false,
            );
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
    /// libpq key/value secret for `role` reaching 127.0.0.1:`port`; `host` is
    /// the name TLS verifies.
    fn secret_at(&self, role: &str, host: &str, port: &str, tls: &str) -> String {
        format!(
            "host={host} hostaddr=127.0.0.1 port={port} user={role} dbname={role} password={PASSWORD} {tls}"
        )
    }
    fn process_at(&self, command: &str, host: &str, port: &str, tls: &str) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        c.args(["db", command, "--target", "test"])
            .env("DB_TARGET", self.secret_at(&self.target, host, port, tls))
            .env("DB_SCRATCH", self.secret_at(&self.scratch, host, port, tls))
            .env_remove("DB_APPROVAL")
            // Default trust is the host CA store only.
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .env("NTNT_ENV", "development")
            .current_dir(self.root.path());
        c
    }
    fn process(&self, command: &str, host: &str, tls: &str) -> Command {
        self.process_at(command, host, &self.port, tls)
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
        fixture_sql(
            &format!(
                "host=127.0.0.1 port={} user={} dbname={} password={PASSWORD} sslmode=disable",
                self.port, self.target, self.target
            ),
            "SELECT to_regclass('public.accounts') IS NOT NULL",
            true,
        )
        .unwrap()
    }
}
impl Drop for TlsFixture {
    fn drop(&mut self) {
        for name in [&self.target, &self.scratch] {
            // One statement per call: a multi-statement batch runs as a single
            // implicit transaction, which DROP DATABASE refuses. Each call is
            // bounded by HANG_GUARD.
            for sql in [
                format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
                format!("DROP ROLE IF EXISTS {name}"),
            ] {
                let _ = std::panic::catch_unwind(|| fixture_sql(&self.admin, &sql, false));
            }
        }
    }
}

/// A session-preserving TLS terminator, like a pooler or cloud proxy in front of
/// PostgreSQL: it answers the SSLRequest, terminates TLS with the fixture's
/// server certificate, and forwards the session to the server in plaintext.
fn start_tls_terminator(server_port: u16, material: &Path) -> u16 {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let certificates = CertificateDer::pem_file_iter(material.join("server.crt"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_file(material.join("server.key")).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            while let Ok((mut client, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    // SSLRequest: length 8, request code 80877103.
                    let mut request = [0u8; 8];
                    if client.read_exact(&mut request).await.is_err()
                        || request != [0, 0, 0, 8, 4, 210, 22, 47]
                        || client.write_all(b"S").await.is_err()
                    {
                        return;
                    }
                    let Ok(mut tls) = acceptor.accept(client).await else {
                        return;
                    };
                    let Ok(mut server) =
                        tokio::net::TcpStream::connect(("127.0.0.1", server_port)).await
                    else {
                        return;
                    };
                    let _ = tokio::io::copy_bidirectional(&mut tls, &mut server).await;
                });
            }
        });
    });
    port
}

fn tls_material(ca: &str) -> PathBuf {
    // The fixture script writes ca.crt, server.crt and server.key side by side.
    Path::new(ca).parent().unwrap().to_path_buf()
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

fn assert_tls(transport: &Value, sslmode: &str) {
    assert_eq!(transport["sslmode"], sslmode, "{transport}");
    assert!(
        transport["tls_version"]
            .as_str()
            .is_some_and(|v| v.starts_with("TLSv1.")),
        "{transport}"
    );
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn verify_full_runs_the_whole_workflow_and_plans_bind_the_transport() {
    let f = TlsFixture::new();
    let tls = format!("sslmode=verify-full sslrootcert={}", f.ca);
    let status = f.run("status", "127.0.0.1", &tls);
    assert_eq!(status["outcome"], "pass", "{status}");
    let transport = &status["target"]["identity"]["transport"];
    assert_tls(transport, "verify-full");
    assert_eq!(transport["roots"], "file");
    assert_eq!(
        transport["roots_sha256"],
        hex::encode(Sha256::digest(std::fs::read(&f.ca).unwrap()))
    );
    // Directly connected, PostgreSQL also reports this session as SSL.
    assert_eq!(transport["server_ssl"], true, "{transport}");
    assert_eq!(
        status["scratch"]["identity"]["transport"]["server_ssl"],
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
fn verify_full_works_through_a_tls_terminating_proxy() {
    let f = TlsFixture::new();
    let server_port: u16 = f.port.parse().unwrap();
    let proxy = start_tls_terminator(server_port, &tls_material(&f.ca)).to_string();
    let tls = format!("sslmode=verify-full sslrootcert={}", f.ca);
    let status = f.json(f.process_at("status", "127.0.0.1", &proxy, &tls));
    assert_eq!(status["outcome"], "pass", "{status}");
    let transport = &status["target"]["identity"]["transport"];
    // The client verified the proxy's certificate and negotiated TLS; the
    // server sees the proxy's plaintext leg.
    assert_tls(transport, "verify-full");
    assert_eq!(transport["server_ssl"], false, "{transport}");
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
    assert_tls(&chain_only["target"]["identity"]["transport"], "verify-ca");
    // Neither an unrelated CA nor the host CA store (the default) trusts the
    // fixture CA.
    for tls in [
        format!("sslmode=verify-full sslrootcert={}", f.other_ca),
        format!("sslmode=verify-ca sslrootcert={}", f.other_ca),
        "sslmode=verify-full".to_string(),
    ] {
        let message = tls_failure(&f.run("status", "127.0.0.1", &tls));
        assert!(message.contains("trusted CA"), "{tls}: {message}");
    }
    // Nothing ran: no plan was possible, so nothing could be migrated.
    assert!(!f.table_exists());
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn verify_ca_never_trusts_the_host_ca_store() {
    let f = TlsFixture::new();
    // The fixture CA stands in for a public CA in the host store, and its
    // certificate for 127.0.0.1 for one issued to someone else's name.
    let with_host_store = |host: &str, tls: &str| {
        let mut status = f.process("status", host, tls);
        status.env("SSL_CERT_FILE", &f.ca);
        f.json(status)
    };
    let trusted = with_host_store("127.0.0.1", "sslmode=verify-full");
    assert_eq!(trusted["outcome"], "pass", "{trusted}");
    assert_eq!(
        trusted["target"]["identity"]["transport"]["roots"],
        "system"
    );
    let message = tls_failure(&with_host_store(
        "db.ntnt-tls.invalid",
        "sslmode=verify-full",
    ));
    assert!(message.contains("host name"), "{message}");
    // Without the name check, host-store roots would accept that certificate.
    for tls in ["sslmode=verify-ca", "sslmode=verify-ca sslrootcert=system"] {
        let r = with_host_store("db.ntnt-tls.invalid", tls);
        assert_ne!(r["outcome"], "pass", "{tls}: {r}");
        assert!(r["target"]["identity"].is_null(), "{tls}: {r}");
        assert!(
            r["diagnostics"].as_array().unwrap().iter().any(|d| {
                d["code"] == "DB_CONNECTION_CONFIG"
                    && d["message"].as_str().unwrap().contains("sslrootcert=")
            }),
            "{tls}: {r}"
        );
    }
}

#[test]
#[ignore = "requires the TLS PostgreSQL fixture; mandatory CI db catalog lane"]
fn require_is_encrypted_but_reported_as_unverified() {
    let f = TlsFixture::new();
    let status = f.run("status", "127.0.0.1", "sslmode=require");
    assert_eq!(status["outcome"], "pass", "{status}");
    let transport = &status["target"]["identity"]["transport"];
    assert_tls(transport, "require");
    assert_eq!(transport["roots"], Value::Null);
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
    // A trailing '&' is accepted, as the driver accepts it.
    let url = |role: &str| {
        format!(
            "postgresql://{role}:{PASSWORD}@127.0.0.1:{}/{role}?application_name=ignored&sslmode=verify-full&sslrootcert={}&",
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
    assert_tls(&status["target"]["identity"]["transport"], "verify-full");
}
