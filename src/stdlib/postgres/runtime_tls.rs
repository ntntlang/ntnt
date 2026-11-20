//! Runtime TLS policy for PostgreSQL: `std/db/postgres` pools and every auth-store
//! connection.
//!
//! An explicit `sslmode` means what it means to `ntnt db`: `verify-full` checks the
//! certificate chain and host name, `verify-ca` the chain only (with
//! `sslrootcert`), `require` encrypts without verifying the server, `disable` is
//! plaintext, and `prefer`/`allow` are rejected because they can fall back to
//! plaintext. Unlike `ntnt db`, a missing `sslmode` is allowed: the connection is
//! plaintext when every host is local (a loopback address, `localhost`, a Unix
//! socket or a single-label name such as a Docker service) and verified TLS
//! (`verify-full` against the host CA store) otherwise; an `sslrootcert` alone also
//! selects `verify-full`. A TLS mode never falls back to plaintext, and errors never
//! echo the connection string.
//!
//! A server that accepted TLS must finish the handshake within `connect_timeout`
//! (10 seconds when unset), or the connection is closed with an error. A target
//! given only as `hostaddr` is named by its address, so TLS verifies the IP.
use crate::migrations::postgres_tls::{self, MakeRustls, Mode, Transport};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_postgres::config::{Host, SslMode};
use zeroize::Zeroizing;

/// A runtime connection string split into the driver's remaining connection
/// string (still secret) and the transport it selects.
pub(crate) struct Target {
    rest: Zeroizing<String>,
    transport: Transport,
}

/// Resolve the transport of a runtime connection string. The error is a
/// secret-free sentence suitable for an application-visible `Err`.
pub(crate) fn resolve(connection_string: &str) -> Result<Target, String> {
    let (rest, transport) = postgres_tls::split_with_default(connection_string, |rest| {
        // A string the driver cannot parse fails later with the driver's own
        // error; until then the safe assumption is a remote host.
        Ok(match rest.parse::<tokio_postgres::Config>() {
            Ok(config) if every_host_is_local(config.get_hosts(), config.get_hostaddrs()) => {
                Mode::Disable
            }
            _ => Mode::VerifyFull,
        })
    })
    .map_err(|diagnostic| diagnostic.message)?;
    Ok(Target { rest, transport })
}

/// Whether every endpoint is local: each host is a Unix socket, `localhost`, a
/// loopback address or a single-label name, and each `hostaddr` is a loopback
/// address. A `hostaddr` decides where the connection goes and a host name what
/// the certificate must match, so both must be local. Vacuously true without
/// hosts (deadpool then defaults to the local Unix sockets, or 127.0.0.1).
pub(crate) fn every_host_is_local(hosts: &[Host], hostaddrs: &[IpAddr]) -> bool {
    hosts.iter().all(host_is_local) && hostaddrs.iter().all(is_loopback)
}

fn host_is_local(host: &Host) -> bool {
    match host {
        Host::Tcp(name) => name_is_local(name),
        #[cfg(unix)]
        Host::Unix(_) => true,
    }
}

fn name_is_local(name: &str) -> bool {
    let unbracketed = name
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(name);
    match unbracketed.parse::<IpAddr>() {
        Ok(address) => is_loopback(&address),
        Err(_) => name.eq_ignore_ascii_case("localhost") || is_single_label(name),
    }
}

fn is_loopback(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// A single-label host name, such as a Docker Compose service (`db`, `postgres`):
/// it starts with a letter and has only letters, digits, `-` and `_`. Anything
/// with a dot (including a trailing one) is not, and neither are the dotless
/// numeric forms (`2130706433`, `0x7f000001`) that resolvers read as IPv4.
fn is_single_label(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Host names for an address-only target (`hostaddr` without `host`): the
/// addresses themselves. tokio-postgres takes the TLS server name from `host`
/// alone, so without them TLS has no name to verify (and deadpool would add its
/// local default hosts). The certificate must then name the IP address, as for
/// `host=<ip>`. An explicit `host` is kept: it names the certificate, while
/// `hostaddr` only decides where to connect.
fn address_hosts(config: &tokio_postgres::Config) -> Option<Vec<String>> {
    (config.get_hosts().is_empty() && !config.get_hostaddrs().is_empty()).then(|| {
        config
            .get_hostaddrs()
            .iter()
            .map(IpAddr::to_string)
            .collect()
    })
}

/// How long a runtime connection gives a server that accepted TLS to finish the
/// handshake: the connection string's `connect_timeout` when set (libpq applies
/// it to the whole attempt, TLS included, but tokio-postgres only to opening the
/// socket), otherwise `postgres_tls::TLS_HANDSHAKE_TIMEOUT`. Each host or
/// address tried gets its own bound, as with libpq's per-host connect_timeout.
pub(crate) fn handshake_timeout(config: &tokio_postgres::Config) -> Duration {
    config
        .get_connect_timeout()
        .copied()
        .unwrap_or(postgres_tls::TLS_HANDSHAKE_TIMEOUT)
}

impl Target {
    pub(crate) fn mode(&self) -> Mode {
        self.transport.mode()
    }
    /// The remaining connection string for the driver (TLS keywords removed).
    pub(crate) fn rest(&self) -> &str {
        self.rest.as_str()
    }
    /// The effective transport, for pool identity: mode and trust source.
    pub(crate) fn policy(&self) -> String {
        self.transport.policy()
    }
    /// The driver mode. It must always be set explicitly: removing the TLS
    /// keywords leaves tokio-postgres's default `prefer`, a silent plaintext
    /// fallback. Every TLS mode is `Require`, which never falls back.
    pub(crate) fn ssl_mode(&self) -> SslMode {
        if self.transport.is_tls() {
            SslMode::Require
        } else {
            SslMode::Disable
        }
    }
    /// Reject TLS over a Unix socket: PostgreSQL serves sockets in plaintext only.
    pub(crate) fn check_hosts(&self, hosts: &[Host]) -> Result<(), String> {
        #[cfg(unix)]
        if self.transport.is_tls() && hosts.iter().any(|host| matches!(host, Host::Unix(_))) {
            return Err(format!(
                "sslmode={} needs TCP hosts. A Unix-socket directory is a local transport: use sslmode=disable (or omit sslmode) for it.",
                self.mode().as_str()
            ));
        }
        #[cfg(not(unix))]
        let _ = hosts;
        Ok(())
    }
    /// The driver configuration, with address-only hosts named and the TLS mode
    /// set explicitly.
    pub(crate) fn driver_config(&self) -> Result<tokio_postgres::Config, String> {
        let mut config = self
            .rest
            .parse::<tokio_postgres::Config>()
            .map_err(|error| format!("Invalid PostgreSQL connection string: {error}"))?;
        for host in address_hosts(&config).unwrap_or_default() {
            config.host(&host);
        }
        self.finish(config)
    }
    /// The pool's driver configuration: deadpool's defaults (user from $USER, a
    /// required dbname, the local Unix sockets or 127.0.0.1 when neither a host
    /// nor an address is given) applied to the TLS-stripped string, with
    /// address-only hosts named and the TLS mode set explicitly.
    pub(crate) fn pool_config(&self) -> Result<tokio_postgres::Config, String> {
        let mut pool = deadpool_postgres::Config::new();
        pool.url = Some(self.rest().to_string());
        // Named before deadpool applies its defaults, which would otherwise
        // add local hosts that neither match the addresses nor name them.
        pool.hosts = self
            .rest
            .parse::<tokio_postgres::Config>()
            .ok()
            .as_ref()
            .and_then(address_hosts);
        let config = pool.get_pg_config().map_err(|error| error.to_string())?;
        self.finish(config)
    }
    fn finish(&self, mut config: tokio_postgres::Config) -> Result<tokio_postgres::Config, String> {
        self.check_hosts(config.get_hosts())?;
        config.ssl_mode(self.ssl_mode());
        Ok(config)
    }
    /// The rustls connector for a TLS mode (`None` for plaintext), loading trust
    /// off the runtime thread under a deadline. Each handshake may take at most
    /// `handshake` (see `handshake_timeout`).
    pub(crate) async fn connector(
        &self,
        handshake: Duration,
    ) -> Result<Option<MakeRustls>, String> {
        self.transport
            .connector()
            .await
            .map(|(connector, _)| connector.map(|tls| tls.with_handshake_timeout(handshake)))
            .map_err(|diagnostic| diagnostic.message)
    }
    /// The same connector for a caller outside any Tokio runtime (the auth
    /// store); trust loads under the same deadline.
    pub(crate) fn connector_blocking(
        &self,
        handshake: Duration,
    ) -> Result<Option<MakeRustls>, String> {
        self.transport
            .connector_blocking()
            .map(|connector| connector.map(|tls| tls.with_handshake_timeout(handshake)))
            .map_err(|diagnostic| diagnostic.message)
    }
}

/// A connect failure as a secret-free sentence: a specific TLS diagnostic when
/// TLS negotiation failed, otherwise the driver's message (which never contains
/// the connection string).
pub(crate) fn connect_error(error: &tokio_postgres::Error) -> String {
    match postgres_tls::classify(error) {
        Some(diagnostic) => format!("TLS: {}", diagnostic.message),
        None => error.to_string(),
    }
}

/// A synchronous connection's parsed configuration and connector.
struct Prepared {
    config: postgres::Config,
    tls: Option<MakeRustls>,
}

/// Open a synchronous client (the auth store) under the runtime TLS policy.
/// The configuration and trust anchors are resolved once per connection string,
/// so a request does not re-read `sslrootcert`; restart after changing trust.
/// A TLS handshake gets `handshake_timeout`: the sync driver waits for the
/// connection with no deadline of its own beyond `connect_timeout` on the socket.
pub(crate) fn connect_sync(connection_string: &str) -> Result<postgres::Client, String> {
    let prepared = prepared(connection_string)?;
    let connected = match &prepared.tls {
        None => prepared.config.connect(postgres::NoTls),
        Some(tls) => prepared.config.connect(tls.clone()),
    };
    connected.map_err(|error| connect_error(&error))
}

fn prepared(connection_string: &str) -> Result<Arc<Prepared>, String> {
    // One slot: the auth store uses one connection string per process.
    static LAST: Mutex<Option<([u8; 32], Arc<Prepared>)>> = Mutex::new(None);
    let digest: [u8; 32] = Sha256::digest(connection_string.as_bytes()).into();
    if let Some((key, prepared)) = &*LAST.lock().unwrap_or_else(|e| e.into_inner()) {
        if *key == digest {
            return Ok(prepared.clone());
        }
    }
    let target = resolve(connection_string)?;
    let config = target.driver_config()?;
    let prepared = Arc::new(Prepared {
        tls: target.connector_blocking(handshake_timeout(&config))?,
        config: config.into(),
    });
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((digest, prepared.clone()));
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(connection_string: &str) -> Mode {
        resolve(connection_string)
            .unwrap_or_else(|error| panic!("{connection_string}: {error}"))
            .mode()
    }
    /// An absolute path on this platform, written as a connection string holds
    /// it (forward slashes: a backslash escapes in key/value form).
    fn absolute(name: &str) -> String {
        if cfg!(windows) {
            format!("C:/ntnt/{name}")
        } else {
            format!("/etc/ntnt/{name}")
        }
    }
    fn rejected(connection_string: &str) -> String {
        match resolve(connection_string) {
            Ok(target) => panic!("{connection_string}: accepted as {:?}", target.mode()),
            Err(error) => error,
        }
    }

    #[test]
    fn local_hosts_without_sslmode_are_plaintext() {
        for local in [
            "host=127.0.0.1 user=u dbname=d",
            "host=127.8.9.10 user=u dbname=d",
            "host=::1 user=u dbname=d",
            "host=localhost user=u dbname=d",
            "host=LOCALHOST user=u dbname=d",
            "host=db user=u dbname=d",
            "host=postgres user=u dbname=d",
            "host=my_db-1 user=u dbname=d",
            "host=db hostaddr=127.0.0.1 user=u dbname=d",
            "hostaddr=127.0.0.1 user=u dbname=d",
            "hostaddr=::ffff:127.0.0.1 user=u dbname=d",
            "host=localhost,db,127.0.0.1 user=u dbname=d",
            "user=u dbname=d",
            "postgres://u@localhost/d",
            "postgres://u@[::1]:5433/d",
            "postgresql://u@db:5432/d?application_name=x",
            "postgres://u@db,localhost/d",
            "postgres:///d?host=localhost",
        ] {
            assert_eq!(mode(local), Mode::Disable, "{local}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_sockets_without_sslmode_are_plaintext() {
        for local in [
            "host=/var/run/postgresql user=u dbname=d",
            "host=/tmp,localhost user=u dbname=d",
            "postgres://u@%2Fvar%2Frun%2Fpostgresql/d",
            "postgres:///d?host=/var/run/postgresql",
        ] {
            assert_eq!(mode(local), Mode::Disable, "{local}");
        }
    }

    #[test]
    fn remote_hosts_without_sslmode_use_verified_tls() {
        for remote in [
            "host=db.example.com user=u dbname=d",
            "host=db.example.test hostaddr=127.0.0.1 user=u dbname=d",
            "host=10.0.0.5 user=u dbname=d",
            "host=192.168.1.20 user=u dbname=d",
            "host=2001:db8::1 user=u dbname=d",
            "host=0.0.0.0 user=u dbname=d",
            "host=db hostaddr=10.0.0.5 user=u dbname=d",
            "hostaddr=10.0.0.5 user=u dbname=d",
            "host=localhost,db.example.com user=u dbname=d",
            "host=db,10.1.2.3 user=u dbname=d",
            "host=db. user=u dbname=d",
            "host=localhost. user=u dbname=d",
            "host=sub.localhost user=u dbname=d",
            "host=2130706433 user=u dbname=d",
            "host=0x7f000001 user=u dbname=d",
            "host=1db user=u dbname=d",
            "postgres://u@db.example.com/d",
            "postgres://u@[2001:db8::1]/d",
            "postgres://u@localhost,db.example.com/d",
            "postgres://u@db.example.test/d?hostaddr=127.0.0.1",
        ] {
            assert_eq!(mode(remote), Mode::VerifyFull, "{remote}");
        }
        let target = resolve("host=db.example.com user=u dbname=d").unwrap();
        assert_eq!(target.policy(), "verify-full roots=system");
        assert_eq!(target.ssl_mode(), SslMode::Require);
    }

    #[test]
    fn explicit_sslmode_wins_over_the_default() {
        assert_eq!(
            mode("host=db.example.com sslmode=disable user=u dbname=d"),
            Mode::Disable
        );
        assert_eq!(
            mode("host=127.0.0.1 sslmode=require user=u dbname=d"),
            Mode::Require
        );
        assert_eq!(
            mode("host=localhost sslmode=verify-full user=u dbname=d"),
            Mode::VerifyFull
        );
        let encoded = absolute("ca.pem").replace(':', "%3A").replace('/', "%2F");
        assert_eq!(
            mode(&format!(
                "postgres://u@db/d?sslmode=verify-ca&sslrootcert={encoded}"
            )),
            Mode::VerifyCa
        );
        // A CA file is a request for verification, never ignored for plaintext.
        let ca = absolute("ca.pem");
        let target = resolve(&format!("host=127.0.0.1 sslrootcert={ca} user=u dbname=d")).unwrap();
        assert_eq!(target.mode(), Mode::VerifyFull);
        assert_eq!(target.policy(), format!("verify-full roots=file:{ca}"));
        assert_eq!(
            mode("host=db sslrootcert=system user=u dbname=d"),
            Mode::VerifyFull
        );
        let disabled = resolve("host=db.example.com sslmode=disable user=u dbname=d").unwrap();
        assert_eq!(disabled.ssl_mode(), SslMode::Disable);
        assert_eq!(disabled.policy(), "disable");
    }

    #[test]
    fn downgradable_and_unknown_modes_are_rejected() {
        for downgradable in [
            "host=localhost sslmode=prefer user=u dbname=d",
            "host=db.example.com sslmode=allow user=u dbname=d",
            "postgres://u@db.example.com/d?sslmode=prefer",
        ] {
            assert!(
                rejected(downgradable).contains("fall back to plaintext"),
                "{downgradable}"
            );
        }
        for invalid in [
            "host=h sslmode=verify user=u dbname=d",
            "host=h sslmode=verify-ca user=u dbname=d",
            "host=h sslmode=disable sslrootcert=/ca.pem user=u dbname=d",
            "host=h sslrootcert=relative.pem user=u dbname=d",
            "host=h sslcert=/client.pem user=u dbname=d",
            "host=h sslmode=disable sslmode=require user=u dbname=d",
        ] {
            rejected(invalid);
        }
    }

    #[test]
    fn errors_never_echo_the_connection_string() {
        for secret in [
            "host=h password=hunter2 sslmode=prefer user=u dbname=d",
            "host=h password=hunter2 sslmode=bogus",
            "postgres://u:hunter2@h/d?sslmode=allow",
            "postgres://u:hunter2@h/d?sslrootcert=hunter2.pem",
            "host=h password='hunter2",
        ] {
            let error = rejected(secret);
            assert!(!error.contains("hunter2"), "{error}");
        }
        let target = resolve("host=h port=x password=hunter2 sslmode=disable").unwrap();
        let error = target.driver_config().unwrap_err();
        assert!(!error.contains("hunter2"), "{error}");
    }

    #[test]
    fn driver_config_always_sets_the_mode_explicitly() {
        // Without TLS keywords tokio-postgres would default to prefer.
        let tls = resolve("host=db.example.com user=u dbname=d").unwrap();
        assert_eq!(
            tls.driver_config().unwrap().get_ssl_mode(),
            SslMode::Require
        );
        let local = resolve("host=127.0.0.1 user=u dbname=d").unwrap();
        assert_eq!(
            local.driver_config().unwrap().get_ssl_mode(),
            SslMode::Disable
        );
        let url =
            resolve("postgres://u:pw@db.example.com/d?sslmode=require&application_name=x").unwrap();
        assert_eq!(
            url.rest(),
            "postgres://u:pw@db.example.com/d?application_name=x"
        );
        let config = url.driver_config().unwrap();
        assert_eq!(config.get_ssl_mode(), SslMode::Require);
        assert_eq!(config.get_application_name(), Some("x"));
    }

    #[test]
    fn address_only_targets_are_named_by_their_addresses() {
        let tcp = |name: &str| Host::Tcp(name.to_string());
        for (secret, hosts) in [
            ("hostaddr=10.0.0.5 user=u dbname=d", vec![tcp("10.0.0.5")]),
            (
                "hostaddr=10.0.0.5,2001:db8::1 user=u dbname=d",
                vec![tcp("10.0.0.5"), tcp("2001:db8::1")],
            ),
            ("postgres://u@/d?hostaddr=10.0.0.5", vec![tcp("10.0.0.5")]),
            (
                "hostaddr=127.0.0.1 sslmode=disable user=u dbname=d",
                vec![tcp("127.0.0.1")],
            ),
            // An explicit host names the certificate; hostaddr only routes.
            (
                "host=db.example.com hostaddr=10.0.0.5 user=u dbname=d",
                vec![tcp("db.example.com")],
            ),
            (
                "host=db.example.com user=u dbname=d",
                vec![tcp("db.example.com")],
            ),
        ] {
            let target = resolve(secret).unwrap();
            for (path, config) in [
                ("driver", target.driver_config().unwrap()),
                ("pool", target.pool_config().unwrap()),
            ] {
                assert_eq!(config.get_hosts(), hosts.as_slice(), "{path}: {secret}");
                let hostaddrs = target
                    .rest()
                    .parse::<tokio_postgres::Config>()
                    .unwrap()
                    .get_hostaddrs()
                    .to_vec();
                assert_eq!(config.get_hostaddrs(), hostaddrs, "{path}: {secret}");
                assert_eq!(config.get_ssl_mode(), target.ssl_mode(), "{path}: {secret}");
            }
        }
        // Without a host or an address, deadpool still supplies its local defaults.
        let pool = resolve("user=u dbname=d").unwrap().pool_config().unwrap();
        assert!(!pool.get_hosts().is_empty());
        assert!(pool.get_hostaddrs().is_empty());
    }

    #[test]
    fn the_tls_handshake_bound_is_connect_timeout_or_ten_seconds() {
        let bound = |secret: &str| {
            let target = resolve(secret).unwrap();
            let bound = handshake_timeout(&target.driver_config().unwrap());
            assert_eq!(
                handshake_timeout(&target.pool_config().unwrap()),
                bound,
                "{secret}"
            );
            bound
        };
        assert_eq!(postgres_tls::TLS_HANDSHAKE_TIMEOUT, Duration::from_secs(10));
        assert_eq!(
            bound("host=db.example.com user=u dbname=d"),
            postgres_tls::TLS_HANDSHAKE_TIMEOUT
        );
        assert_eq!(
            bound("host=db.example.com connect_timeout=3 user=u dbname=d"),
            Duration::from_secs(3)
        );
        assert_eq!(
            bound("postgres://u@db.example.com/d?connect_timeout=25&sslmode=require"),
            Duration::from_secs(25)
        );
        // Zero means "no limit" to libpq; TLS negotiation keeps the default.
        assert_eq!(
            bound("host=db.example.com connect_timeout=0 user=u dbname=d"),
            postgres_tls::TLS_HANDSHAKE_TIMEOUT
        );
    }

    #[cfg(unix)]
    #[test]
    fn tls_over_a_unix_socket_is_rejected() {
        for secret in [
            "host=/var/run/postgresql sslmode=require user=u dbname=d",
            "host=/tmp,db.example.com user=u dbname=d",
        ] {
            let target = resolve(secret).unwrap();
            let error = target.driver_config().unwrap_err();
            assert!(error.contains("Unix-socket"), "{secret}: {error}");
        }
    }
}
