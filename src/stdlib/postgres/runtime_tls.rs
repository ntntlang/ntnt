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
use crate::migrations::postgres_tls::{self, MakeRustls, Mode, Transport};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
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
    /// The driver configuration with the TLS mode set explicitly.
    pub(crate) fn driver_config(&self) -> Result<tokio_postgres::Config, String> {
        let mut config = self
            .rest
            .parse::<tokio_postgres::Config>()
            .map_err(|error| format!("Invalid PostgreSQL connection string: {error}"))?;
        self.check_hosts(config.get_hosts())?;
        config.ssl_mode(self.ssl_mode());
        Ok(config)
    }
    /// The rustls connector for a TLS mode (`None` for plaintext), loading trust
    /// off the runtime thread under a deadline.
    pub(crate) async fn connector(&self) -> Result<Option<MakeRustls>, String> {
        self.transport
            .connector()
            .await
            .map(|(connector, _)| connector)
            .map_err(|diagnostic| diagnostic.message)
    }
    /// The same connector, loading trust on the calling thread.
    pub(crate) fn connector_blocking(&self) -> Result<Option<MakeRustls>, String> {
        self.transport
            .connector_blocking()
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
    let prepared = Arc::new(Prepared {
        config: target.driver_config()?.into(),
        tls: target.connector_blocking()?,
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
