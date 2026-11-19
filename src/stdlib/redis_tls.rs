//! Redis/Valkey connections for `std/kv`, `std/jobs` and the auth store.
//!
//! `redis://` and `valkey://` are plaintext, as written. `rediss://` and
//! `valkeys://` are TLS: the certificate chain must lead to the host CA store
//! (the trust NTNT's other TLS clients use, including `SSL_CERT_FILE` and
//! `SSL_CERT_DIR`) and the certificate must be valid for the URL's host name.
//! There is no unverified mode, and a TLS URL never falls back to plaintext.
//! Every connection has a deadline covering TCP, TLS and the setup commands
//! (see [`connection`]). Errors never echo the URL, which may hold a password.
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Accepted schemes and the redis crate's name for each.
const SCHEMES: [(&str, &str); 4] = [
    ("redis://", "redis://"),
    ("valkey://", "redis://"),
    ("rediss://", "rediss://"),
    ("valkeys://", "rediss://"),
];

/// Whether `url` names a Redis/Valkey server, in plaintext or over TLS.
pub(crate) fn is_redis_url(url: &str) -> bool {
    SCHEMES.iter().any(|(scheme, _)| url.starts_with(scheme))
}

/// The backend label: `valkey` for `valkey://` and `valkeys://`, else `redis`.
pub(crate) fn backend_name(url: &str) -> &'static str {
    if url.starts_with("valkey://") || url.starts_with("valkeys://") {
        "valkey"
    } else {
        "redis"
    }
}

/// The URL with Valkey's schemes mapped to the redis crate's: `valkey://` to
/// `redis://` and `valkeys://` to `rediss://`. Other URLs are unchanged.
pub(crate) fn normalize(url: &str) -> String {
    for (scheme, redis) in SCHEMES {
        if let Some(rest) = url.strip_prefix(scheme) {
            return format!("{redis}{rest}");
        }
    }
    url.to_string()
}

/// A client for `url`. TLS URLs trust exactly the host CA store, read once per
/// process; restart after changing it. The last client is reused, so a store
/// that connects per operation does not rebuild its trust anchors each time.
pub(crate) fn client(url: &str) -> Result<redis::Client, String> {
    static LAST: Mutex<Option<([u8; 32], redis::Client)>> = Mutex::new(None);
    let key: [u8; 32] = Sha256::digest(url.as_bytes()).into();
    if let Some((cached, client)) = &*LAST.lock().unwrap_or_else(|e| e.into_inner()) {
        if *cached == key {
            return Ok(client.clone());
        }
    }
    let client = client_with_roots(url, host_roots_pem)?;
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, client.clone()));
    Ok(client)
}

/// [`client`] with the PEM trust anchors for TLS URLs supplied by `roots`.
fn client_with_roots(
    url: &str,
    roots: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<redis::Client, String> {
    let info = redis::IntoConnectionInfo::into_connection_info(normalize(url).as_str())
        .map_err(|error| format!("Invalid Redis URL: {error}"))?;
    match &info.addr {
        redis::ConnectionAddr::TcpTls { insecure: true, .. } => Err(
            "TLS certificate verification cannot be disabled: remove #insecure from the rediss:// or valkeys:// URL, or use redis:// for plaintext.".to_string(),
        ),
        // Supplied roots replace redis's default store, so verification uses
        // exactly the host CA store. The webpki-roots feature only keeps redis
        // from loading the OS store again on every connect (see Cargo.toml).
        redis::ConnectionAddr::TcpTls { .. } => redis::Client::build_with_tls(
            info,
            redis::TlsCertificates {
                client_tls: None,
                root_cert: Some(roots()?),
            },
        )
        .map_err(|error| format!("Invalid Redis TLS configuration: {error}")),
        _ => redis::Client::open(info).map_err(|error| format!("Invalid Redis URL: {error}")),
    }
}

/// The host CA store as PEM for the redis crate, encoded once per process.
fn host_roots_pem() -> Result<Vec<u8>, String> {
    static PEM: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    PEM.get_or_init(|| crate::stdlib::tls_trust::host_trust().map(|trust| pem(&trust.certificates)))
        .clone()
}

fn pem(certificates: &[rustls::pki_types::CertificateDer<'_>]) -> Vec<u8> {
    use base64::Engine;
    let mut pem = String::new();
    for certificate in certificates {
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        let encoded = base64::engine::general_purpose::STANDARD.encode(certificate.as_ref());
        for line in encoded.as_bytes().chunks(64) {
            // Base64 output is ASCII, so every chunk is valid UTF-8.
            pem.push_str(std::str::from_utf8(line).unwrap_or_default());
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    pem.into_bytes()
}

/// Connect `client` with `timeout` as its deadline. redis (0.28.2 and later)
/// gives the TCP connect `timeout`, then spends the time left as the read and
/// write timeout of the TLS handshake and the setup commands (CLIENT SETINFO,
/// and AUTH and SELECT when the URL sets them). So a server that stops
/// answering at any step fails the attempt instead of hanging it, and the
/// attempt's socket is closed. Each wait is bounded, not their sum: a server
/// that stalls after the handshake times out each pipelined setup reply in turn
/// (two by default). A TLS failure is reported as a specific, secret-free
/// sentence.
pub(crate) fn connection(
    client: &redis::Client,
    timeout: Duration,
) -> Result<redis::Connection, String> {
    client.get_connection_with_timeout(timeout).map_err(|error| {
        if error.is_timeout() {
            format!("connection timed out: no answer within {timeout:?} during the TCP connect, TLS handshake or setup commands")
        } else {
            describe(&error)
        }
    })
}

/// The connect deadline (see [`connection`]) for callers that set none of
/// their own: `std/kv` `open`, the job store and the auth session store. Each
/// step of a connection waits one round trip: a few milliseconds inside a
/// region, a few hundred milliseconds across continents. So 10 seconds fails
/// only a server that has stopped answering, while bounding how long a request
/// that needs the auth store waits for one.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The deadline for connections whose caller sets none: [`CONNECT_TIMEOUT`],
/// or in unit tests a shorter one set for the current thread.
pub(crate) fn default_connect_timeout() -> Duration {
    #[cfg(test)]
    if let Some(timeout) = TEST_CONNECT_TIMEOUT.with(std::cell::Cell::get) {
        return timeout;
    }
    CONNECT_TIMEOUT
}

#[cfg(test)]
thread_local! {
    static TEST_CONNECT_TIMEOUT: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

/// Run `f` with `timeout` as this thread's [`default_connect_timeout`], so a
/// test that must hit the default deadline need not wait the full 10 seconds.
#[cfg(test)]
pub(crate) fn with_default_connect_timeout<T>(timeout: Duration, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Duration>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_CONNECT_TIMEOUT.with(|cell| cell.set(self.0));
        }
    }
    let _restore = Restore(TEST_CONNECT_TIMEOUT.with(|cell| cell.replace(Some(timeout))));
    f()
}

/// A Redis error as a sentence; TLS certificate failures get specific advice.
pub(crate) fn describe(error: &redis::RedisError) -> String {
    // RedisError keeps the handshake's io::Error private (no `source`), so the
    // rustls certificate error is recognized by its display text: the io::Error
    // shows the wrapped `rustls::Error`, "invalid peer certificate: <reason>".
    let text = error.to_string();
    let Some(reason) = text.strip_prefix("invalid peer certificate: ") else {
        return text;
    };
    let is = |prefixes: &[&str]| prefixes.iter().any(|prefix| reason.starts_with(prefix));
    if is(&["NotValidForName", "certificate not valid for name"]) {
        "TLS: the server certificate is not valid for the host name in the URL."
    } else if is(&["UnknownIssuer", "BadSignature"]) {
        "TLS: the server certificate is not issued by a CA in the host CA store (set SSL_CERT_FILE or SSL_CERT_DIR for a private CA)."
    } else if is(&[
        "Expired",
        "certificate expired",
        "NotValidYet",
        "certificate not valid yet",
    ]) {
        "TLS: the server certificate is expired or not yet valid."
    } else {
        "TLS: the server certificate was rejected."
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_map_valkey_to_redis_names() {
        for (url, normalized, backend) in [
            ("redis://h:6379/0", "redis://h:6379/0", "redis"),
            ("rediss://h:6380/0", "rediss://h:6380/0", "redis"),
            ("valkey://h:6379/0", "redis://h:6379/0", "valkey"),
            ("valkeys://h:6380/0", "rediss://h:6380/0", "valkey"),
        ] {
            assert!(is_redis_url(url), "{url}");
            assert_eq!(normalize(url), normalized);
            assert_eq!(backend_name(url), backend);
        }
        for other in [
            "sqlite:./kv.db",
            "./kv.db",
            ":memory:",
            "redis:/h",
            "unix:///tmp/r.sock",
        ] {
            assert!(!is_redis_url(other), "{other}");
            assert_eq!(normalize(other), other);
        }
    }

    #[test]
    fn tls_schemes_build_tls_clients_and_plain_schemes_do_not() {
        let roots = || Ok(pem(&[]));
        for url in ["rediss://127.0.0.1:6380/0", "valkeys://127.0.0.1:6380/0"] {
            let client = client_with_roots(url, roots).unwrap();
            assert!(
                matches!(
                    client.get_connection_info().addr,
                    redis::ConnectionAddr::TcpTls {
                        insecure: false,
                        ..
                    }
                ),
                "{url}"
            );
        }
        for url in ["redis://127.0.0.1:6379/0", "valkey://127.0.0.1:6379/0"] {
            let client =
                client_with_roots(url, || panic!("a plaintext URL loads no trust")).unwrap();
            assert!(
                matches!(
                    client.get_connection_info().addr,
                    redis::ConnectionAddr::Tcp(..)
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn insecure_tls_is_rejected_before_connecting() {
        for url in [
            "rediss://127.0.0.1:6380/#insecure",
            "valkeys://:secret@127.0.0.1:6380/0#insecure",
        ] {
            let error = client_with_roots(url, || panic!("no trust is loaded")).unwrap_err();
            assert!(error.contains("#insecure"), "{url}: {error}");
            assert!(!error.contains("secret"), "{error}");
        }
    }

    #[test]
    fn invalid_urls_are_not_echoed() {
        for url in [
            "rediss://:hunter2@/0",
            "redis://:hunter2@h:notaport/0",
            "valkeys://:hunter2@h/notadb",
        ] {
            let error = client_with_roots(url, || Ok(pem(&[]))).unwrap_err();
            assert!(!error.contains("hunter2"), "{url}: {error}");
        }
    }

    /// The error a rustls handshake failure becomes in a sync redis connection.
    fn handshake_error(error: rustls::Error) -> redis::RedisError {
        redis::RedisError::from(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    #[test]
    fn certificate_failures_are_described() {
        use rustls::pki_types::{ServerName, UnixTime};
        use rustls::CertificateError as Certificate;
        let certificate =
            |error| describe(&handshake_error(rustls::Error::InvalidCertificate(error)));
        let name = ServerName::try_from("127.0.0.1").unwrap();
        for wrong_name in [
            Certificate::NotValidForName,
            Certificate::NotValidForNameContext {
                expected: name,
                presented: vec!["DnsName(\"redis.example.test\")".into()],
            },
        ] {
            assert!(certificate(wrong_name).contains("not valid for the host name"));
        }
        for untrusted in [Certificate::UnknownIssuer, Certificate::BadSignature] {
            assert!(certificate(untrusted).contains("not issued by a CA in the host CA store"));
        }
        let time = UnixTime::since_unix_epoch(std::time::Duration::from_secs(2_000_000_000));
        let earlier = UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_000_000_000));
        for stale in [
            Certificate::Expired,
            Certificate::ExpiredContext {
                time,
                not_after: earlier,
            },
            Certificate::NotValidYet,
            Certificate::NotValidYetContext {
                time: earlier,
                not_before: time,
            },
        ] {
            assert!(certificate(stale).contains("expired or not yet valid"));
        }
        assert!(certificate(Certificate::Revoked).contains("was rejected"));
        // Anything else keeps the driver's own message.
        let alert = handshake_error(rustls::Error::AlertReceived(
            rustls::AlertDescription::HandshakeFailure,
        ));
        assert_eq!(describe(&alert), alert.to_string());
        let refused =
            redis::RedisError::from(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
        assert_eq!(describe(&refused), refused.to_string());
    }

    use crate::stdlib::redis_test_server::{self as stand_in, Event, Plan, HANG_GUARD};

    /// The stand-in never answers, so a deadline of any length is hit; this one
    /// leaves the steps before the stall (TCP connect, and for a stalled setup
    /// the TLS handshake) at least 10x the time they normally take.
    const DEADLINE: Duration = Duration::from_secs(1);

    /// Connect on another thread, so a missing deadline fails the test at the
    /// hang guard instead of hanging it.
    fn connect_in_background(client: redis::Client) -> std::sync::mpsc::Receiver<String> {
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let error = match connection(&client, DEADLINE) {
                Ok(_) => "connected to a stand-in that never answers".to_string(),
                Err(error) => error,
            };
            let _ = done.send(error);
        });
        result
    }

    fn timeout_error(attempt: std::sync::mpsc::Receiver<String>) -> String {
        attempt.recv_timeout(HANG_GUARD).unwrap_or_else(|_| {
            panic!("still connecting at the hang guard: the deadline does not cover this stage")
        })
    }

    #[test]
    fn the_connect_deadline_covers_a_stalled_tls_handshake() {
        let server = stand_in::start(vec![Plan::StallHandshake], None);
        let url = format!("rediss://127.0.0.1:{}/0", server.port);
        let attempt = connect_in_background(client_with_roots(&url, || Ok(pem(&[]))).unwrap());
        // A TLS record (the ClientHello) arrived: TCP is done, TLS is stalled.
        let event = server.next();
        assert!(
            matches!(event, Event::Stalled { first: 0x16, .. }),
            "{event:?}"
        );
        let error = timeout_error(attempt);
        assert!(error.contains("timed out: no answer within 1s"), "{error}");
        // The abandoned attempt is closed, not left open.
        let event = server.next();
        assert!(matches!(event, Event::ClosedByClient { .. }), "{event:?}");
    }

    #[test]
    fn the_connect_deadline_covers_setup_commands_after_the_tls_handshake() {
        let (config, ca) = stand_in::tls_identity();
        let server = stand_in::start(vec![Plan::StallSetup], Some(config));
        let url = format!("rediss://127.0.0.1:{}/0", server.port);
        let client = client_with_roots(&url, || Ok(pem(&[ca]))).unwrap();
        let attempt = connect_in_background(client);
        // A RESP command arrived inside TLS: the handshake completed.
        let event = server.next();
        assert!(
            matches!(event, Event::Stalled { first: b'*', .. }),
            "{event:?}"
        );
        let error = timeout_error(attempt);
        assert!(error.contains("timed out: no answer within 1s"), "{error}");
        let event = server.next();
        assert!(matches!(event, Event::ClosedByClient { .. }), "{event:?}");
    }

    #[test]
    fn the_connect_deadline_covers_plaintext_setup_commands() {
        let server = stand_in::start(vec![Plan::StallSetup], None);
        let url = format!("redis://127.0.0.1:{}/0", server.port);
        let attempt = connect_in_background(client_with_roots(&url, || unreachable!()).unwrap());
        let event = server.next();
        assert!(
            matches!(event, Event::Stalled { first: b'*', .. }),
            "{event:?}"
        );
        let error = timeout_error(attempt);
        assert!(error.contains("timed out: no answer within 1s"), "{error}");
        let event = server.next();
        assert!(matches!(event, Event::ClosedByClient { .. }), "{event:?}");
    }

    #[test]
    fn connections_without_their_own_deadline_get_ten_seconds() {
        assert_eq!(default_connect_timeout(), Duration::from_secs(10));
        let short = Duration::from_millis(5);
        assert_eq!(
            with_default_connect_timeout(short, default_connect_timeout),
            short
        );
        assert_eq!(default_connect_timeout(), Duration::from_secs(10));
    }

    #[test]
    fn pem_round_trips_through_the_redis_parser() {
        use rustls::pki_types::pem::PemObject;
        let ca = rcgen::generate_simple_self_signed(vec!["ca.ntnt-tls.test".into()]).unwrap();
        let encoded = pem(&[ca.cert.der().clone()]);
        let parsed: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(&encoded)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(parsed, vec![ca.cert.der().clone()]);
    }
}
