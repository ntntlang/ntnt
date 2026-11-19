//! Runtime TLS for `std/db/postgres`, the auth store and `std/kv` (Redis/Valkey),
//! against in-process fake servers that record what their client sent, so each
//! test proves which transport was used. No real PostgreSQL or Redis is needed.
//! The host CA store is selected with `SSL_CERT_FILE` in child `ntnt run`
//! processes only, never by changing this process's environment.
#[path = "support/bounded_output.rs"]
mod bounded_output;

use bounded_output::BoundedOutput;
use ntnt::interpreter::Value;
use ntnt::stdlib::auth::SessionStore;
use ntnt::stdlib::auth::{ensure_auth_session_store, parse_auth_session_store, AuthConfig};
use ntnt::stdlib::postgres as pg;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Only a hang reaches this; every exchange here takes milliseconds.
const HANG_GUARD: Duration = Duration::from_secs(60);
/// PostgreSQL SSLRequest: length 8, request code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

fn pg_connect(url: &str) -> Result<Value, String> {
    let Some(Value::NativeFunction { func, .. }) = pg::init().remove("connect") else {
        unreachable!("std/db/postgres exports connect")
    };
    match func(&[Value::String(url.into())]).unwrap() {
        Value::EnumValue {
            variant,
            mut values,
            ..
        } if variant == "Ok" => Ok(values.remove(0)),
        Value::EnumValue {
            variant, values, ..
        } if variant == "Err" => match &values[0] {
            Value::String(message) => Err(message.clone()),
            other => panic!("expected an error string, got {other:?}"),
        },
        other => panic!("expected a Result, got {other:?}"),
    }
}

fn auth_store(url: String) -> Result<(), String> {
    ensure_auth_session_store(&AuthConfig {
        session_store: SessionStore::Postgres(url),
        ..AuthConfig::default()
    })
}

/// What a fake PostgreSQL server saw on one connection.
#[derive(Debug, PartialEq, Eq)]
enum Opening {
    /// An SSLRequest, answered `N`; the client then closed without sending more.
    RefusedThenClosed,
    /// An SSLRequest answered `N`, after which the client still sent these bytes.
    SentAfterRefusal(Vec<u8>),
    /// The first 8 bytes, which were not an SSLRequest.
    Plaintext([u8; 8]),
}

/// A plaintext PostgreSQL stand-in: it answers an SSLRequest with `N` (no TLS)
/// and reports, per connection in order, what the client did. A client that
/// does not ask for TLS gets its connection closed after 8 bytes.
fn refusing_server() -> (u16, mpsc::Receiver<Opening>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, openings) = mpsc::channel();
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let mut client = client.unwrap();
            let mut first = [0u8; 8];
            if client.read_exact(&mut first).is_err() {
                continue;
            }
            if first != SSL_REQUEST {
                let _ = report.send(Opening::Plaintext(first));
                continue;
            }
            client.write_all(b"N").unwrap();
            let mut rest = [0u8; 256];
            let _ = report.send(match client.read(&mut rest) {
                Ok(0) | Err(_) => Opening::RefusedThenClosed,
                Ok(n) => Opening::SentAfterRefusal(rest[..n].to_vec()),
            });
        }
    });
    (port, openings)
}

fn next<T>(reports: &mpsc::Receiver<T>) -> T {
    reports
        .recv_timeout(HANG_GUARD)
        .unwrap_or_else(|error| panic!("the fake server reported nothing: {error}"))
}

/// A plaintext StartupMessage starts with its length and protocol major version 3.
fn is_startup(first: &[u8; 8]) -> bool {
    first[4..6] == [0, 3]
}

#[test]
fn pool_without_sslmode_never_downgrades_a_named_host() {
    let (port, openings) = refusing_server();
    // hostaddr reaches the fake server; the multi-label name is what TLS verifies.
    let error = pg_connect(&format!(
        "host=db.example.test hostaddr=127.0.0.1 port={port} user=ntnt dbname=ntnt"
    ))
    .unwrap_err();
    assert_eq!(next(&openings), Opening::RefusedThenClosed, "{error}");
    assert!(error.contains("does not accept TLS"), "{error}");
}

#[test]
fn auth_store_without_sslmode_never_downgrades_a_named_host() {
    let (port, openings) = refusing_server();
    let error = auth_store(format!(
        "postgres://ntnt@db.example.test:{port}/ntnt?hostaddr=127.0.0.1"
    ))
    .unwrap_err();
    assert_eq!(next(&openings), Opening::RefusedThenClosed, "{error}");
    assert!(error.contains("does not accept TLS"), "{error}");
}

#[test]
fn explicit_tls_modes_never_fall_back_even_on_loopback() {
    for tls in ["sslmode=require", "sslmode=verify-full"] {
        let (port, openings) = refusing_server();
        let error = pg_connect(&format!(
            "host=127.0.0.1 port={port} user=ntnt dbname=ntnt {tls}"
        ))
        .unwrap_err();
        assert_eq!(
            next(&openings),
            Opening::RefusedThenClosed,
            "{tls}: {error}"
        );
        assert!(error.contains("does not accept TLS"), "{tls}: {error}");
        let (port, openings) = refusing_server();
        let error = auth_store(format!("postgres://ntnt@127.0.0.1:{port}/ntnt?{tls}")).unwrap_err();
        assert_eq!(
            next(&openings),
            Opening::RefusedThenClosed,
            "{tls}: {error}"
        );
        assert!(error.contains("does not accept TLS"), "{tls}: {error}");
    }
}

#[test]
fn local_hosts_without_sslmode_use_plaintext() {
    for host in ["host=127.0.0.1", "host=db hostaddr=127.0.0.1"] {
        let (port, openings) = refusing_server();
        let _ = pg_connect(&format!("{host} port={port} user=ntnt dbname=ntnt"));
        let opening = next(&openings);
        assert!(
            matches!(opening, Opening::Plaintext(ref first) if is_startup(first)),
            "{host}: {opening:?}"
        );
    }
    let (port, openings) = refusing_server();
    let _ = auth_store(format!("postgres://ntnt@127.0.0.1:{port}/ntnt"));
    let opening = next(&openings);
    assert!(
        matches!(opening, Opening::Plaintext(ref first) if is_startup(first)),
        "{opening:?}"
    );
}

#[test]
fn prefer_and_allow_are_rejected_before_connecting() {
    let (port, openings) = refusing_server();
    for mode in ["prefer", "allow"] {
        let error = pg_connect(&format!(
            "host=127.0.0.1 port={port} user=ntnt dbname=ntnt sslmode={mode}"
        ))
        .unwrap_err();
        assert!(error.contains("fall back to plaintext"), "{mode}: {error}");
        let error = auth_store(format!(
            "postgres://ntnt@127.0.0.1:{port}/ntnt?sslmode={mode}"
        ))
        .unwrap_err();
        assert!(error.contains("fall back to plaintext"), "{mode}: {error}");
    }
    // Connections are reported in order, so if this sentinel is the first one
    // the server saw, nothing above connected.
    TcpStream::connect(("127.0.0.1", port))
        .unwrap()
        .write_all(b"sentinel")
        .unwrap();
    assert_eq!(next(&openings), Opening::Plaintext(*b"sentinel"));
}

#[test]
fn connection_errors_never_echo_the_connection_string() {
    let (port, openings) = refusing_server();
    for secret in [
        format!("host=db.example.test hostaddr=127.0.0.1 port={port} user=ntnt password=hunter2-canary dbname=ntnt"),
        format!("host=127.0.0.1 port={port} user=ntnt password=hunter2-canary dbname=ntnt sslmode=prefer"),
        "host=127.0.0.1 password='hunter2-canary".to_string(),
    ] {
        let error = pg_connect(&secret).unwrap_err();
        assert!(!error.contains("hunter2-canary"), "{error}");
    }
    assert_eq!(next(&openings), Opening::RefusedThenClosed);
    let error = auth_store(format!(
        "postgres://ntnt:hunter2-canary@db.example.test:{port}/ntnt?hostaddr=127.0.0.1"
    ))
    .unwrap_err();
    assert!(!error.contains("hunter2-canary"), "{error}");
    assert_eq!(next(&openings), Opening::RefusedThenClosed);
}

// ---------------------------------------------------------------------------
// Verified TLS through the host CA store, in child `ntnt run` processes.
// ---------------------------------------------------------------------------

/// A throwaway CA and a server certificate it issued for `names` (rcgen encodes
/// `127.0.0.1` as an IP SAN), as a rustls server configuration.
struct Pki {
    ca: PathBuf,
    server: Arc<rustls::ServerConfig>,
    _dir: tempfile::TempDir,
}

fn pki(names: &[&str]) -> Pki {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "ntnt runtime TLS test CA");
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let mut leaf_params = CertificateParams::new(
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "ntnt runtime TLS test server");
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        PrivatePkcs8KeyDer::from(leaf_key.serialize_der()).into(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ca.pem");
    std::fs::write(&path, ca.pem()).unwrap();
    Pki {
        ca: path,
        server: Arc::new(server),
        _dir: dir,
    }
}

/// Run `body` with `ntnt run`, trusting exactly the CA file `trust` as the host
/// CA store. Returns stdout and stderr together.
fn run_script(body: &str, trust: &Path, variables: &[(&str, String)]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("tls.tnt");
    std::fs::write(&script, body).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command
        .arg("run")
        .arg(&script)
        .current_dir(dir.path())
        .env("SSL_CERT_FILE", trust)
        .env_remove("SSL_CERT_DIR")
        .env_remove("NTNT_ENV")
        .env_remove("NTNT_SECRETS_PROVIDER");
    for (key, value) in variables {
        command.env(key, value);
    }
    let output = command.bounded_output();
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// What a fake TLS PostgreSQL server saw on one connection.
#[derive(Debug, PartialEq, Eq)]
enum PgSeen {
    /// TLS was negotiated and this user's StartupMessage arrived over it.
    TlsStartup(String),
    /// The client asked for TLS but the handshake did not complete.
    HandshakeFailed,
    /// The client sent these 8 bytes instead of an SSLRequest.
    Plaintext([u8; 8]),
}

/// A TLS PostgreSQL stand-in: it accepts the SSLRequest, completes the TLS
/// handshake, reads the StartupMessage and answers with an ErrorResponse, which
/// the client reports as `db error`. That reply exists only inside TLS.
fn tls_postgres_server(config: Arc<rustls::ServerConfig>) -> (u16, mpsc::Receiver<PgSeen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, seen) = mpsc::channel();
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let mut client = client.unwrap();
            let mut first = [0u8; 8];
            if client.read_exact(&mut first).is_err() {
                continue;
            }
            if first != SSL_REQUEST {
                let _ = report.send(PgSeen::Plaintext(first));
                continue;
            }
            client.write_all(b"S").unwrap();
            let connection = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, client);
            let mut length = [0u8; 4];
            if tls.read_exact(&mut length).is_err() {
                let _ = report.send(PgSeen::HandshakeFailed);
                continue;
            }
            let mut body = vec![0u8; u32::from_be_bytes(length) as usize - 4];
            tls.read_exact(&mut body).unwrap();
            let fields = "SFATAL\0VFATAL\0C08006\0Mntnt tls fixture\0\0";
            let mut message = vec![b'E'];
            message.extend_from_slice(&(4 + fields.len() as u32).to_be_bytes());
            message.extend_from_slice(fields.as_bytes());
            tls.write_all(&message).unwrap();
            tls.flush().unwrap();
            // Parameters follow the 4-byte protocol version as NUL-terminated pairs.
            let parameters: Vec<&[u8]> = body[4..].split(|b| *b == 0).collect();
            let user = parameters
                .chunks(2)
                .find(|pair| pair[0] == b"user")
                .map(|pair| String::from_utf8_lossy(pair[1]).into_owned())
                .unwrap_or_default();
            let _ = report.send(PgSeen::TlsStartup(user));
        }
    });
    (port, seen)
}

const PG_SCRIPT: &str = r#"
import { connect, close } from "std/db/postgres"
import { get_env } from "std/env"

match connect(unwrap(get_env("NTNT_TLS_TEST_PG"))) {
    Ok(db) => {
        print("connected")
        close(db)
    },
    Err(e) => print("connect error: #{e}")
}
"#;

#[test]
fn pool_without_sslmode_verifies_a_named_host_against_the_host_ca_store() {
    let trusted = pki(&["pg.ntnt-tls.test"]);
    let (port, seen) = tls_postgres_server(trusted.server.clone());
    let url = format!(
        "host=pg.ntnt-tls.test hostaddr=127.0.0.1 port={port} user=ntnt_tls dbname=ntnt_tls"
    );
    let output = run_script(PG_SCRIPT, &trusted.ca, &[("NTNT_TLS_TEST_PG", url.clone())]);
    // The server's ErrorResponse can only arrive over the verified TLS session.
    assert!(
        output.contains("connect error: Connection failed: db error"),
        "{output}"
    );
    assert_eq!(next(&seen), PgSeen::TlsStartup("ntnt_tls".into()));

    // A host CA store without the issuing CA rejects the same server.
    let unrelated = pki(&["pg.ntnt-tls.test"]);
    let output = run_script(PG_SCRIPT, &unrelated.ca, &[("NTNT_TLS_TEST_PG", url)]);
    assert!(output.contains("not issued by a trusted CA"), "{output}");
    assert_eq!(next(&seen), PgSeen::HandshakeFailed);

    // verify-full checks the name: a certificate for another name is refused.
    let url = format!(
        "host=other.ntnt-tls.test hostaddr=127.0.0.1 port={port} user=ntnt_tls dbname=ntnt_tls"
    );
    let output = run_script(PG_SCRIPT, &trusted.ca, &[("NTNT_TLS_TEST_PG", url)]);
    assert!(
        output.contains("not valid for the connection host name"),
        "{output}"
    );
    assert_eq!(next(&seen), PgSeen::HandshakeFailed);
}

// ---------------------------------------------------------------------------
// Address-only targets (`hostaddr` without `host`) and bounded TLS negotiation.
// ---------------------------------------------------------------------------

/// A path as a key/value connection-string value: forward slashes, because a
/// backslash escapes there.
fn conninfo_path(path: &Path) -> String {
    path.to_str().unwrap().replace('\\', "/")
}

/// The same path percent-encoded for a URL query.
fn url_path(path: &Path) -> String {
    conninfo_path(path)
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[test]
fn pool_names_an_address_only_target_by_its_address() {
    // The certificate names IP 127.0.0.1 and nothing else.
    let pki = pki(&["127.0.0.1"]);
    let (port, seen) = tls_postgres_server(pki.server.clone());
    let ca = conninfo_path(&pki.ca);
    for tls in [
        format!("sslmode=verify-full sslrootcert='{ca}'"),
        "sslmode=require".to_string(),
    ] {
        let error = pg_connect(&format!(
            "hostaddr=127.0.0.1 port={port} user=ntnt_tls dbname=ntnt_tls {tls}"
        ))
        .unwrap_err();
        // The server's ErrorResponse can only arrive over the TLS session.
        assert!(error.contains("db error"), "{tls}: {error}");
        assert_eq!(next(&seen), PgSeen::TlsStartup("ntnt_tls".into()), "{tls}");
    }
    // An explicit host still names the certificate, and this one does not match.
    let error = pg_connect(&format!(
        "host=pg.ntnt-tls.test hostaddr=127.0.0.1 port={port} user=ntnt_tls dbname=ntnt_tls sslmode=verify-full sslrootcert='{ca}'"
    ))
    .unwrap_err();
    assert!(
        error.contains("not valid for the connection host name"),
        "{error}"
    );
    assert_eq!(next(&seen), PgSeen::HandshakeFailed);
}

#[test]
fn auth_store_names_an_address_only_target_by_its_address() {
    let pki = pki(&["127.0.0.1"]);
    let (port, seen) = tls_postgres_server(pki.server.clone());
    let ca = url_path(&pki.ca);
    for tls in [
        format!("sslmode=verify-full&sslrootcert={ca}"),
        "sslmode=require".to_string(),
    ] {
        // `postgres://user@/db`: no host, only the address.
        let error = auth_store(format!(
            "postgres://ntnt_tls@/ntnt_tls?hostaddr=127.0.0.1&port={port}&{tls}"
        ))
        .unwrap_err();
        assert!(error.contains("db error"), "{tls}: {error}");
        assert_eq!(next(&seen), PgSeen::TlsStartup("ntnt_tls".into()), "{tls}");
    }
    let error = auth_store(format!(
        "postgres://ntnt_tls@pg.ntnt-tls.test:{port}/ntnt_tls?hostaddr=127.0.0.1&sslmode=verify-full&sslrootcert={ca}"
    ))
    .unwrap_err();
    assert!(
        error.contains("not valid for the connection host name"),
        "{error}"
    );
    assert_eq!(next(&seen), PgSeen::HandshakeFailed);
}

#[test]
fn pool_reaches_a_local_address_only_target_in_plaintext() {
    let (port, openings) = refusing_server();
    let _ = pg_connect(&format!(
        "hostaddr=127.0.0.1 port={port} user=ntnt dbname=ntnt"
    ));
    // Connections are reported in order: the attempt above comes before this.
    TcpStream::connect(("127.0.0.1", port))
        .unwrap()
        .write_all(b"sentinel")
        .unwrap();
    let opening = next(&openings);
    assert!(
        matches!(opening, Opening::Plaintext(ref first) if is_startup(first)),
        "{opening:?}"
    );
    assert_eq!(next(&openings), Opening::Plaintext(*b"sentinel"));
}

/// What a server that stalls TLS negotiation saw on one connection.
#[derive(Debug, PartialEq, Eq)]
enum Stalled {
    /// It answered the SSLRequest with `S` and received a TLS ClientHello (a
    /// handshake record, 0x16), never replied, and the client then closed the
    /// connection.
    HelloThenClosed,
    /// After `S` the client sent these bytes, which do not start a TLS record.
    NotTls(Vec<u8>),
    /// The first 8 bytes, which were not an SSLRequest.
    Plaintext([u8; 8]),
}

/// A PostgreSQL stand-in that accepts TLS (answers the SSLRequest with `S`)
/// and then never continues the handshake. It holds each connection until the
/// client closes it and reports, per connection in order, what it received.
fn stalling_tls_server() -> (u16, mpsc::Receiver<Stalled>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, seen) = mpsc::channel();
    std::thread::spawn(move || {
        for client in listener.incoming() {
            let mut client = client.unwrap();
            let mut first = [0u8; 8];
            if client.read_exact(&mut first).is_err() {
                continue;
            }
            if first != SSL_REQUEST {
                let _ = report.send(Stalled::Plaintext(first));
                continue;
            }
            client.write_all(b"S").unwrap();
            let mut received = Vec::new();
            let _ = client.read_to_end(&mut received);
            let _ = report.send(if received.first() == Some(&0x16) {
                Stalled::HelloThenClosed
            } else {
                Stalled::NotTls(received)
            });
        }
    });
    (port, seen)
}

/// Run a connection attempt on its own thread, so an attempt that hangs fails
/// the test at the hang guard instead of hanging it.
fn bounded<T: Send + 'static>(attempt: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(attempt());
    });
    result
        .recv_timeout(HANG_GUARD)
        .expect("the connection attempt hung after the server accepted TLS")
}

#[test]
fn pool_fails_when_the_server_stalls_tls_negotiation() {
    let (port, seen) = stalling_tls_server();
    // connect_timeout also bounds TLS negotiation; 1 s keeps the test short.
    let url = format!(
        "host=127.0.0.1 port={port} user=ntnt dbname=ntnt sslmode=require connect_timeout=1"
    );
    let error = bounded(move || pg_connect(&url).map(|_| ())).unwrap_err();
    // The diagnostic names the bound in force: connect_timeout, not the default.
    assert!(
        error.contains("did not complete the TLS handshake within 1s"),
        "{error}"
    );
    // The client discarded the connection ...
    assert_eq!(next(&seen), Stalled::HelloThenClosed, "{error}");
    // ... and opened no other one (a plaintext retry would arrive before this).
    TcpStream::connect(("127.0.0.1", port))
        .unwrap()
        .write_all(b"sentinel")
        .unwrap();
    assert_eq!(next(&seen), Stalled::Plaintext(*b"sentinel"));
}

#[test]
fn auth_store_fails_when_the_server_stalls_tls_negotiation() {
    let (port, seen) = stalling_tls_server();
    let url = format!("postgres://ntnt@127.0.0.1:{port}/ntnt?sslmode=require&connect_timeout=1");
    let error = bounded(move || auth_store(url)).unwrap_err();
    assert!(
        error.contains("did not complete the TLS handshake within 1s"),
        "{error}"
    );
    assert_eq!(next(&seen), Stalled::HelloThenClosed, "{error}");
    TcpStream::connect(("127.0.0.1", port))
        .unwrap()
        .write_all(b"sentinel")
        .unwrap();
    assert_eq!(next(&seen), Stalled::Plaintext(*b"sentinel"));
}

// ---------------------------------------------------------------------------
// Redis/Valkey: an in-process tokio-rustls listener with a minimal RESP responder.
// ---------------------------------------------------------------------------

/// What a fake Redis server saw on one connection.
#[derive(Debug, PartialEq, Eq)]
enum RedisSeen {
    /// TLS completed and these commands (first word only) arrived over it.
    Tls(Vec<String>),
    /// The TLS handshake did not complete.
    HandshakeFailed,
    /// A plaintext listener received these commands.
    Plain(Vec<String>),
    /// A plaintext listener received a TLS record (0x16, a ClientHello) instead of RESP.
    TlsRecordOnPlaintext,
}

fn redis_server(tls: Option<Arc<rustls::ServerConfig>>) -> (u16, mpsc::Receiver<RedisSeen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (report, seen) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            while let Ok((stream, _)) = listener.accept().await {
                let (report, tls) = (report.clone(), tls.clone());
                tokio::spawn(async move {
                    let seen = match tls {
                        Some(config) => {
                            match tokio_rustls::TlsAcceptor::from(config).accept(stream).await {
                                Ok(stream) => RedisSeen::Tls(serve_resp(stream).await),
                                Err(_) => RedisSeen::HandshakeFailed,
                            }
                        }
                        None => {
                            let mut first = [0u8; 1];
                            match stream.peek(&mut first).await {
                                Ok(1) if first[0] == 0x16 => RedisSeen::TlsRecordOnPlaintext,
                                _ => RedisSeen::Plain(serve_resp(stream).await),
                            }
                        }
                    };
                    let _ = report.send(seen);
                });
            }
        });
    });
    (port, seen)
}

/// Answer RESP commands until the client closes: PING, SET/GET/DEL on an
/// in-memory map, an empty SCAN, and `+OK` for setup commands (CLIENT, SELECT).
/// Returns the command names in order.
async fn serve_resp<S>(stream: S) -> Vec<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let mut stream = BufReader::new(stream);
    let mut data = std::collections::HashMap::<Vec<u8>, Vec<u8>>::new();
    let mut names = Vec::new();
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
            return names;
        }
        let Some(count) = line
            .trim_end()
            .strip_prefix('*')
            .and_then(|n| n.parse().ok())
        else {
            return names;
        };
        let mut words: Vec<Vec<u8>> = Vec::with_capacity(count);
        for _ in 0..count {
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            let length: usize = line.trim_end()[1..].parse().unwrap();
            let mut word = vec![0u8; length + 2];
            stream.read_exact(&mut word).await.unwrap();
            word.truncate(length);
            words.push(word);
        }
        let name = String::from_utf8_lossy(&words[0]).to_ascii_uppercase();
        let reply: Vec<u8> = match name.as_str() {
            "PING" => b"+PONG\r\n".to_vec(),
            "SET" => {
                data.insert(words[1].clone(), words[2].clone());
                b"+OK\r\n".to_vec()
            }
            "GET" => match data.get(&words[1]) {
                Some(value) => {
                    let mut reply = format!("${}\r\n", value.len()).into_bytes();
                    reply.extend_from_slice(value);
                    reply.extend_from_slice(b"\r\n");
                    reply
                }
                None => b"$-1\r\n".to_vec(),
            },
            "DEL" => {
                let removed = words[1..]
                    .iter()
                    .filter(|k| data.remove(*k).is_some())
                    .count();
                format!(":{removed}\r\n").into_bytes()
            }
            "SCAN" => b"*2\r\n$1\r\n0\r\n*0\r\n".to_vec(),
            _ => b"+OK\r\n".to_vec(),
        };
        names.push(name);
        if stream.get_mut().write_all(&reply).await.is_err() {
            return names;
        }
    }
}

const KV_SCRIPT: &str = r#"
import { open, set, get as kv_get } from "std/kv"
import { get_env } from "std/env"

let kv = unwrap(open(unwrap(get_env("NTNT_TLS_TEST_KV"))))
set(kv, "greeting", "hello over tls")
let value = unwrap(kv_get(kv, "greeting")) ?? "missing"
print("read: #{value}")
"#;

#[test]
fn kv_rediss_and_valkeys_verify_the_certificate_against_the_host_ca_store() {
    let trusted = pki(&["127.0.0.1"]);
    let (port, seen) = redis_server(Some(trusted.server.clone()));
    for scheme in ["rediss", "valkeys"] {
        let url = format!("{scheme}://127.0.0.1:{port}/0");
        let output = run_script(KV_SCRIPT, &trusted.ca, &[("NTNT_TLS_TEST_KV", url)]);
        assert!(
            output.contains("read: hello over tls"),
            "{scheme}: {output}"
        );
        match next(&seen) {
            RedisSeen::Tls(commands) => {
                assert!(
                    commands.iter().any(|c| c == "SET"),
                    "{scheme}: {commands:?}"
                )
            }
            other => panic!("{scheme}: {other:?}"),
        }
    }
    let unrelated = pki(&["127.0.0.1"]);
    let url = format!("rediss://127.0.0.1:{port}/0");
    let output = run_script(KV_SCRIPT, &unrelated.ca, &[("NTNT_TLS_TEST_KV", url)]);
    assert!(output.contains("not issued by a CA"), "{output}");
    assert_eq!(next(&seen), RedisSeen::HandshakeFailed);
}

/// One host CA entry that rustls cannot parse must not break rediss://: the
/// host CA store skips it, and Redis verifies against those roots only.
#[test]
fn kv_rediss_ignores_a_malformed_host_ca_entry() {
    let trusted = pki(&["127.0.0.1"]);
    let (port, seen) = redis_server(Some(trusted.server.clone()));
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("host-store.pem");
    let mut bundle = std::fs::read_to_string(&trusted.ca).unwrap();
    // Valid PEM framing around bytes that are not a certificate.
    bundle.push_str("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n");
    std::fs::write(&store, bundle).unwrap();
    let url = format!("rediss://127.0.0.1:{port}/0");
    let output = run_script(KV_SCRIPT, &store, &[("NTNT_TLS_TEST_KV", url)]);
    assert!(output.contains("read: hello over tls"), "{output}");
    match next(&seen) {
        RedisSeen::Tls(commands) => assert!(commands.iter().any(|c| c == "SET"), "{commands:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn kv_rediss_checks_the_host_name() {
    // Trusted CA, but the certificate names another host than the URL's 127.0.0.1.
    let pki = pki(&["redis.ntnt-tls.test"]);
    let (port, seen) = redis_server(Some(pki.server.clone()));
    let url = format!("rediss://127.0.0.1:{port}/0");
    let output = run_script(KV_SCRIPT, &pki.ca, &[("NTNT_TLS_TEST_KV", url)]);
    assert!(output.contains("not valid for the host name"), "{output}");
    assert_eq!(next(&seen), RedisSeen::HandshakeFailed);
}

#[test]
fn kv_redis_stays_plaintext_and_rediss_never_falls_back() {
    let trust = pki(&["127.0.0.1"]);
    let (port, seen) = redis_server(None);
    let output = run_script(
        KV_SCRIPT,
        &trust.ca,
        &[("NTNT_TLS_TEST_KV", format!("redis://127.0.0.1:{port}/0"))],
    );
    assert!(output.contains("read: hello over tls"), "{output}");
    assert!(
        matches!(next(&seen), RedisSeen::Plain(ref commands) if commands.iter().any(|c| c == "SET"))
    );
    // A plaintext server receives a ClientHello and nothing in plaintext.
    let output = run_script(
        KV_SCRIPT,
        &trust.ca,
        &[("NTNT_TLS_TEST_KV", format!("rediss://127.0.0.1:{port}/0"))],
    );
    assert!(output.contains("Failed to connect to Redis"), "{output}");
    assert!(!output.contains("read:"), "{output}");
    assert_eq!(next(&seen), RedisSeen::TlsRecordOnPlaintext);
}

#[test]
fn auth_session_store_accepts_rediss_and_valkeys() {
    for url in [
        "rediss://cache.example.test:6380/0",
        "valkeys://cache.example.test:6380/0",
    ] {
        assert!(
            matches!(parse_auth_session_store(url), Ok(SessionStore::Redis(_))),
            "{url}"
        );
    }
}

#[test]
fn auth_redis_store_uses_verified_tls() {
    let trusted = pki(&["127.0.0.1"]);
    let (port, seen) = redis_server(Some(trusted.server.clone()));
    let script = r#"
import { enable_auth } from "std/auth"
import { get_env } from "std/env"
enable_auth([], map { "session_store": unwrap(get_env("NTNT_TLS_TEST_AUTH")), "session_secret": "fixture-only-session-secret", "cookie_secure": false })
print("auth store ready")
"#;
    let url = format!("valkeys://127.0.0.1:{port}/0");
    let output = run_script(script, &trusted.ca, &[("NTNT_TLS_TEST_AUTH", url.clone())]);
    // Logged only after the store initialized (connect, PING, legacy-key scan).
    assert!(output.contains("Using Valkey session storage"), "{output}");
    assert!(
        matches!(next(&seen), RedisSeen::Tls(ref commands) if commands.iter().any(|c| c == "PING")),
    );
    let unrelated = pki(&["127.0.0.1"]);
    let output = run_script(script, &unrelated.ca, &[("NTNT_TLS_TEST_AUTH", url)]);
    assert!(output.contains("Failed to connect to Redis"), "{output}");
    assert!(output.contains("not issued by a CA"), "{output}");
    assert!(!output.contains("Using Valkey session storage"), "{output}");
    assert_eq!(next(&seen), RedisSeen::HandshakeFailed);
}
