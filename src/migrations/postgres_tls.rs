//! TLS transport for PostgreSQL sessions: `ntnt db`, and through
//! `crate::stdlib::postgres::runtime_tls` the `std/db/postgres` pools and the
//! auth store.
//!
//! tokio-postgres accepts only `sslmode=disable|prefer|require` and leaves
//! certificate policy to its connector. This module reads libpq's `sslmode`
//! (including `verify-ca` and `verify-full`) and `sslrootcert` from the
//! connection secret, removes them before tokio-postgres parses the rest, and
//! supplies a rustls connector that enforces the chosen verification. Every TLS
//! mode becomes tokio-postgres `SslMode::Require`, so a server without TLS is an
//! error, never a plaintext fallback.
use super::{Diagnostic, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::{CertificateError, DigitallySignedStruct, RootCertStore, SignatureScheme};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::io::{self, Read};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};
use tokio_postgres::Socket;
use zeroize::Zeroizing;

/// Largest accepted `sslrootcert` file. CA bundles are far smaller.
const MAX_ROOTS_BYTES: u64 = 1_048_576;
/// Reading trust anchors runs off the runtime thread under this deadline, so a
/// slow filesystem cannot stall a command beyond its other bounds.
const ROOTS_TIMEOUT: Duration = Duration::from_secs(5);
/// Default bound on each TLS handshake after the server accepted TLS (answered
/// the SSLRequest with `S`), so a server that stops the exchange is an error,
/// not a hang. Runtime connections use the connection string's
/// `connect_timeout` instead when it is set. `ntnt db` bounds its whole connect
/// at 5 seconds, which always ends first, so this never decides its outcome.
pub(crate) const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The transport requested by the connection secret's `sslmode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Plaintext; only for a trusted local transport.
    Disable,
    /// Encrypted, but the server's identity is not verified. Explicit opt-in.
    Require,
    /// Encrypted; the certificate chain must lead to a trusted root. Host name not checked.
    VerifyCa,
    /// Encrypted; trusted chain and a certificate valid for the connection host name.
    VerifyFull,
}
impl Mode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Mode::Disable => "disable",
            Mode::Require => "require",
            Mode::VerifyCa => "verify-ca",
            Mode::VerifyFull => "verify-full",
        }
    }
}

/// Where verify-ca/verify-full trust anchors come from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum RootSource {
    /// The host CA store (or SSL_CERT_FILE / SSL_CERT_DIR), shared with NTNT's other TLS clients.
    /// verify-full only: verify-ca does not check the host name.
    System,
    /// An absolute path to a PEM file holding the only trusted CA certificates.
    File(PathBuf),
}

/// Requested transport, parsed from the connection secret before connecting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Transport {
    mode: Mode,
    roots: Option<RootSource>,
}

fn config_error(message: &str) -> Diagnostic {
    Diagnostic::new("DB_CONNECTION_CONFIG", message)
}
fn invalid_secret() -> Diagnostic {
    config_error("Invalid connection string; details suppressed.")
}
fn roots_timed_out() -> Diagnostic {
    config_error("Reading sslrootcert or the host CA store did not finish in time.")
}
fn verify_ca_needs_file() -> Diagnostic {
    config_error(
        "sslmode=verify-ca needs sslrootcert=/absolute/path/ca.pem naming the CA that issued the server certificate. verify-ca does not check the host name, so with the host CA store it would accept any certificate those CAs issued, for any name. Use sslmode=verify-full with the host CA store.",
    )
}

/// The remaining (still secret) connection string and the TLS keyword values.
type Split = (Zeroizing<String>, Vec<(String, String)>);

/// Split `sslmode`/`sslrootcert` out of a libpq key/value string or a
/// `postgres://` URL. Returns the remaining connection string for tokio-postgres
/// (still secret: it holds the password) and the requested transport. Errors
/// never echo connection-string contents. `ntnt db` requires an explicit sslmode.
pub(crate) fn split(secret: &str) -> Result<(Zeroizing<String>, Transport)> {
    let (rest, tls) = split_tls_keywords(secret)?;
    Ok((rest, transport(tls, None)?))
}

/// `split` for runtime connections, where `sslmode` may be absent. Then
/// `sslrootcert` (a CA the caller wants checked) selects verify-full, and
/// otherwise `absent` chooses the mode from the remaining connection string.
/// An explicit sslmode is interpreted exactly as by `split`.
pub(crate) fn split_with_default(
    secret: &str,
    absent: impl Fn(&str) -> Result<Mode>,
) -> Result<(Zeroizing<String>, Transport)> {
    let (rest, tls) = split_tls_keywords(secret)?;
    let transport = transport(tls, Some(&|| absent(rest.as_str())))?;
    Ok((rest, transport))
}

fn split_tls_keywords(secret: &str) -> Result<Split> {
    match ["postgresql://", "postgres://"]
        .iter()
        .find(|prefix| secret.starts_with(**prefix))
    {
        Some(prefix) => split_url(secret, prefix.len()),
        None => split_keywords(secret),
    }
}

/// TLS keywords handled here rather than by tokio-postgres. `sslnegotiation` is
/// left to tokio-postgres; any other `ssl*` keyword is rejected below.
fn is_tls_keyword(key: &str) -> bool {
    key.starts_with("ssl") && key != "sslnegotiation"
}

/// Key/value form, tokenized exactly as tokio-postgres 0.7's `Parser` does so
/// the kept parameters keep their original bytes (and quoting). Only TLS
/// values are decoded; other values (passwords) are never copied out.
fn split_keywords(s: &str) -> Result<Split> {
    let mut kept: Vec<&str> = Vec::new();
    let mut tls = Vec::new();
    let mut it = s.char_indices().peekable();
    let skip_ws = |it: &mut std::iter::Peekable<std::str::CharIndices<'_>>| {
        while matches!(it.peek(), Some(&(_, c)) if c.is_whitespace()) {
            it.next();
        }
    };
    loop {
        skip_ws(&mut it);
        let Some(&(start, _)) = it.peek() else { break };
        let mut key_end = start;
        while let Some(&(i, c)) = it.peek() {
            if c.is_whitespace() || c == '=' {
                break;
            }
            it.next();
            key_end = i + c.len_utf8();
        }
        if key_end == start {
            // tokio-postgres stops at an empty keyword and ignores the rest.
            break;
        }
        let key = &s[start..key_end];
        let tls_key = is_tls_keyword(key);
        skip_ws(&mut it);
        if !matches!(it.next(), Some((_, '='))) {
            return Err(invalid_secret());
        }
        skip_ws(&mut it);
        let mut value = String::new();
        let end;
        if matches!(it.peek(), Some(&(_, '\''))) {
            it.next();
            loop {
                match it.next() {
                    Some((i, '\'')) => {
                        end = i + 1;
                        break;
                    }
                    Some((_, '\\')) => match it.next() {
                        Some((_, c)) if tls_key => value.push(c),
                        Some(_) => {}
                        None => return Err(invalid_secret()),
                    },
                    Some((_, c)) if tls_key => value.push(c),
                    Some(_) => {}
                    None => return Err(invalid_secret()),
                }
            }
        } else {
            // tokio-postgres rejects an empty unquoted value (including a lone
            // trailing backslash), so track whether any character was produced.
            let mut last = None;
            let mut produced = false;
            while let Some(&(i, c)) = it.peek() {
                if c.is_whitespace() {
                    break;
                }
                it.next();
                last = Some(i + c.len_utf8());
                let decoded = if c == '\\' {
                    it.next().map(|(j, escaped)| {
                        last = Some(j + escaped.len_utf8());
                        escaped
                    })
                } else {
                    Some(c)
                };
                if let Some(decoded) = decoded {
                    produced = true;
                    if tls_key {
                        value.push(decoded);
                    }
                }
            }
            end = match last {
                Some(end) if produced => end,
                _ => return Err(invalid_secret()),
            };
        }
        if tls_key {
            tls.push((key.to_string(), value));
        } else {
            kept.push(&s[start..end]);
        }
    }
    Ok((Zeroizing::new(kept.join(" ")), tls))
}

/// URL form: TLS parameters live in the query string, which tokio-postgres
/// splits on `&` after the first `?` following any `user:password@` part.
fn split_url(s: &str, prefix_len: usize) -> Result<Split> {
    let body = &s[prefix_len..];
    let credentials_end = body.find('@').map_or(0, |i| i + 1);
    let Some(question) = body[credentials_end..].find('?') else {
        return Ok((Zeroizing::new(s.to_string()), Vec::new()));
    };
    let query_start = prefix_len + credentials_end + question;
    let mut rest = &s[query_start + 1..];
    let mut kept: Vec<&str> = Vec::new();
    let mut tls = Vec::new();
    // Same walk as tokio-postgres: key up to the next '=', value up to the next
    // '&' (consumed) or the end. A trailing '&' therefore ends the query.
    while !rest.is_empty() {
        let equals = rest.find('=').ok_or_else(invalid_secret)?;
        let after = &rest[equals + 1..];
        let (value, next) = match after.find('&') {
            Some(amp) => (&after[..amp], &after[amp + 1..]),
            None => (after, ""),
        };
        let key = percent_decode(&rest[..equals])?;
        if is_tls_keyword(&key) {
            tls.push((key, percent_decode(value)?));
        } else {
            kept.push(&rest[..equals + 1 + value.len()]);
        }
        rest = next;
    }
    let head = &s[..query_start];
    let rest = if kept.is_empty() {
        Zeroizing::new(head.to_string())
    } else {
        Zeroizing::new(format!("{head}?{}", kept.join("&")))
    };
    Ok((rest, tls))
}

/// Percent-decoding as the `percent-encoding` crate does it: `%XX` with two hex
/// digits becomes that byte; any other `%` is kept. The result must be UTF-8.
fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let hex = |b: Option<&u8>| b.and_then(|b| (*b as char).to_digit(16));
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(high), Some(low)) = (hex(bytes.get(i + 1)), hex(bytes.get(i + 2))) {
                out.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).map_err(|_| invalid_secret())
}

/// Interpret the TLS keywords. Without `absent`, `sslmode` must be explicit;
/// `prefer`/`allow` are always rejected because they can fall back to plaintext.
fn transport(
    tls: Vec<(String, String)>,
    absent: Option<&dyn Fn() -> Result<Mode>>,
) -> Result<Transport> {
    let mut mode: Option<String> = None;
    let mut roots: Option<String> = None;
    for (key, value) in tls {
        let slot = match key.as_str() {
            "sslmode" => &mut mode,
            "sslrootcert" => &mut roots,
            _ => {
                return Err(config_error(
                    "Unsupported TLS keyword in the connection string. Only sslmode and sslrootcert are supported; client certificates, CRLs and protocol-version keywords are not.",
                ))
            }
        };
        if slot.replace(value).is_some() {
            return Err(config_error(
                "sslmode or sslrootcert appears more than once in the connection string.",
            ));
        }
    }
    let mode = match mode.as_deref() {
        Some("disable") => Mode::Disable,
        Some("require") => Mode::Require,
        Some("verify-ca") => Mode::VerifyCa,
        Some("verify-full") => Mode::VerifyFull,
        Some("prefer") | Some("allow") => {
            return Err(config_error(
                "sslmode=prefer and sslmode=allow can fall back to plaintext. Use verify-full, verify-ca, require, or disable on a trusted local transport.",
            ))
        }
        Some(_) => {
            return Err(config_error(
                "Unknown sslmode. Use verify-full, verify-ca, require, or disable.",
            ))
        }
        None => match absent {
            // Like libpq's `sslrootcert=system`, a named CA means verified TLS:
            // it must never be ignored in favor of plaintext.
            Some(_) if roots.is_some() => Mode::VerifyFull,
            Some(default) => default()?,
            None => {
                return Err(config_error(
                    "Set sslmode explicitly: verify-full (recommended), verify-ca, require, or disable on a trusted local transport.",
                ))
            }
        },
    };
    let roots = match (mode, roots) {
        (Mode::VerifyFull, None) => Some(RootSource::System),
        (Mode::VerifyFull, Some(value)) if value == "system" => Some(RootSource::System),
        // Without the host-name check, a host-store CA would vouch for any
        // certificate it issued to anyone, for any name. libpq refuses this too.
        (Mode::VerifyCa, None) => return Err(verify_ca_needs_file()),
        (Mode::VerifyCa, Some(value)) if value == "system" => return Err(verify_ca_needs_file()),
        (Mode::VerifyCa | Mode::VerifyFull, Some(path)) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(config_error(
                    "sslrootcert must be an absolute path to a PEM file of CA certificates, or `system` with sslmode=verify-full.",
                ));
            }
            Some(RootSource::File(path))
        }
        (_, Some(_)) => {
            return Err(config_error(
                "sslrootcert applies only to sslmode=verify-ca or sslmode=verify-full.",
            ))
        }
        (_, None) => None,
    };
    Ok(Transport { mode, roots })
}

/// Trust anchors and, for an sslrootcert file, the SHA-256 of its bytes.
type LoadedRoots = Option<(Arc<RootCertStore>, Option<String>)>;

fn load_roots(source: Option<RootSource>) -> Result<LoadedRoots> {
    match source {
        None => Ok(None),
        Some(RootSource::System) => crate::stdlib::tls_trust::host_trust()
            .map(|trust| Some((trust.roots.clone(), None)))
            .map_err(|message| Diagnostic::new("DB_TLS", &message)),
        Some(RootSource::File(path)) => {
            file_roots(&path).map(|(store, sha256)| Some((Arc::new(store), Some(sha256))))
        }
    }
}

impl Transport {
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }
    /// The mode and trust source, e.g. `verify-full roots=system`: what makes two
    /// connections to the same server equivalent. Contains no secret.
    pub(crate) fn policy(&self) -> String {
        match &self.roots {
            None => self.mode.as_str().to_string(),
            Some(RootSource::System) => format!("{} roots=system", self.mode.as_str()),
            Some(RootSource::File(path)) => {
                format!("{} roots=file:{}", self.mode.as_str(), path.display())
            }
        }
    }
    /// Whether this transport is TLS (every mode except disable).
    pub(crate) fn is_tls(&self) -> bool {
        self.mode != Mode::Disable
    }
    /// Load trust anchors and build the connector, or `None` for plaintext.
    /// Also returns the transport description that identity reports bind.
    /// Filesystem and host-store reads run off the runtime thread with a deadline.
    pub(crate) async fn connector(&self) -> Result<(Option<MakeRustls>, Value)> {
        let description = json!({"sslmode": self.mode.as_str(), "roots": null});
        if self.mode == Mode::Disable {
            return Ok((None, description));
        }
        let source = self.roots.clone();
        let loaded = tokio::time::timeout(
            ROOTS_TIMEOUT,
            tokio::task::spawn_blocking(move || load_roots(source)),
        )
        .await;
        let roots = match loaded {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => return Err(Diagnostic::new("DB_TLS", "TLS configuration failed.")),
            Err(_) => return Err(roots_timed_out()),
        };
        self.build(roots, description)
    }
    /// The same connector for synchronous clients that run outside any Tokio
    /// runtime. Trust loads on a helper thread under the same deadline; a read
    /// that stalls is abandoned, never joined. `None` for plaintext.
    pub(crate) fn connector_blocking(&self) -> Result<Option<MakeRustls>> {
        if self.mode == Mode::Disable {
            return Ok(None);
        }
        let description = json!({"sslmode": self.mode.as_str(), "roots": null});
        let source = self.roots.clone();
        let roots = match with_deadline(ROOTS_TIMEOUT, move || load_roots(source)) {
            Ok(result) => result?,
            Err(RecvTimeoutError::Timeout) => return Err(roots_timed_out()),
            // The loader died without a result (or its thread could not start).
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Diagnostic::new("DB_TLS", "TLS configuration failed."))
            }
        };
        Ok(self.build(roots, description)?.0)
    }
    fn build(
        &self,
        roots: LoadedRoots,
        mut description: Value,
    ) -> Result<(Option<MakeRustls>, Value)> {
        let roots = roots.map(|(store, sha256)| {
            match sha256 {
                Some(sha256) => {
                    description["roots"] = json!("file");
                    description["roots_sha256"] = json!(sha256);
                }
                None => description["roots"] = json!("system"),
            }
            store
        });
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|_| Diagnostic::new("DB_TLS", "TLS configuration failed."))?;
        let mut config = match (self.mode, roots) {
            (Mode::VerifyFull, Some(roots)) => {
                builder.with_root_certificates(roots).with_no_client_auth()
            }
            (Mode::VerifyCa, Some(roots)) if matches!(self.roots, Some(RootSource::File(_))) => {
                builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(ChainOnly { roots, provider }))
                    .with_no_client_auth()
            }
            (Mode::Require, None) => builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(Unverified { provider }))
                .with_no_client_auth(),
            _ => return Err(Diagnostic::new("DB_TLS", "TLS configuration failed.")),
        };
        // PostgreSQL 17+ uses this ALPN identifier; older servers ignore it.
        config.alpn_protocols = vec![b"postgresql".to_vec()];
        Ok((
            Some(MakeRustls {
                config: Arc::new(config),
                negotiated: Arc::default(),
                handshake_timeout: TLS_HANDSHAKE_TIMEOUT,
            }),
            description,
        ))
    }
}

/// Run `work` on its own thread and wait at most `deadline` for its result. On
/// timeout the worker is abandoned, never joined, so a stalled filesystem read
/// cannot hold up the caller. `Disconnected` means it ended without a result.
fn with_deadline<T: Send + 'static>(
    deadline: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> std::result::Result<T, RecvTimeoutError> {
    let (done, result) = std::sync::mpsc::channel();
    // A thread that cannot start drops `done`, which reads as Disconnected.
    let _ = std::thread::Builder::new()
        .name("ntnt-tls-roots".into())
        .spawn(move || {
            let _ = done.send(work());
        });
    result.recv_timeout(deadline)
}

/// Read an `sslrootcert` PEM file: bounded, regular file, at least one usable CA.
fn file_roots(path: &std::path::Path) -> Result<(RootCertStore, String)> {
    let unreadable = || {
        config_error(
            "sslrootcert must name a readable PEM file of CA certificates (at most 1 MiB).",
        )
    };
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // A FIFO without a writer would block a plain open; open without waiting and
    // reject anything that is not a regular file. Symlinks are followed (mounted
    // secrets are often symlinks to the real file).
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| unreadable())?;
    let metadata = file.metadata().map_err(|_| unreadable())?;
    if !metadata.is_file() || metadata.len() > MAX_ROOTS_BYTES {
        return Err(unreadable());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ROOTS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 > MAX_ROOTS_BYTES {
        return Err(unreadable());
    }
    let certificates = CertificateDer::pem_slice_iter(&bytes)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| unreadable())?;
    let mut store = RootCertStore::empty();
    store.add_parsable_certificates(certificates);
    if store.is_empty() {
        return Err(config_error(
            "sslrootcert contains no usable CA certificates.",
        ));
    }
    Ok((store, hex::encode(Sha256::digest(&bytes))))
}

/// verify-ca: the chain must lead to a configured root and be valid for server
/// authentication now. The host name is deliberately not compared.
#[derive(Debug)]
struct ChainOnly {
    roots: Arc<RootCertStore>,
    provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let certificate = rustls::server::ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &certificate,
            &self.roots,
            intermediates,
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// require: encryption without server authentication, chosen explicitly and
/// reported in plans. Handshake signatures are still checked against the
/// presented certificate, which proves key possession, not identity.
#[derive(Debug)]
struct Unverified {
    provider: Arc<CryptoProvider>,
}
impl ServerCertVerifier for Unverified {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// tokio-postgres connector backed by rustls.
#[derive(Clone)]
pub(crate) struct MakeRustls {
    config: Arc<rustls::ClientConfig>,
    /// Protocol version of the last completed handshake, as this client saw it.
    negotiated: Arc<Mutex<Option<&'static str>>>,
    /// Bound on each handshake (each host or address tried gets its own).
    handshake_timeout: Duration,
}
impl MakeRustls {
    /// The TLS version this client negotiated, e.g. "TLSv1.3". A TLS-terminating
    /// proxy in front of PostgreSQL does not change it.
    pub(crate) fn negotiated_version(&self) -> Option<&'static str> {
        *self
            .negotiated
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// The same connector with another bound on each TLS handshake.
    pub(crate) fn with_handshake_timeout(mut self, limit: Duration) -> Self {
        self.handshake_timeout = limit;
        self
    }
}
impl MakeTlsConnect<Socket> for MakeRustls {
    type Stream = RustlsStream;
    type TlsConnect = RustlsConnect;
    type Error = io::Error;
    fn make_tls_connect(&mut self, domain: &str) -> io::Result<RustlsConnect> {
        let name = ServerName::try_from(domain.to_string())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid TLS server name"))?;
        Ok(RustlsConnect {
            config: self.config.clone(),
            name,
            negotiated: self.negotiated.clone(),
            handshake_timeout: self.handshake_timeout,
        })
    }
}
pub(crate) struct RustlsConnect {
    config: Arc<rustls::ClientConfig>,
    name: ServerName<'static>,
    negotiated: Arc<Mutex<Option<&'static str>>>,
    handshake_timeout: Duration,
}
impl TlsConnect<Socket> for RustlsConnect {
    type Stream = RustlsStream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<RustlsStream>> + Send>>;
    fn connect(self, stream: Socket) -> Self::Future {
        Box::pin(async move {
            let limit = self.handshake_timeout;
            let handshake =
                tokio_rustls::TlsConnector::from(self.config).connect(self.name, stream);
            // On timeout the handshake, and with it the socket, is dropped: the
            // connection is discarded. Every TLS mode is the driver's `Require`,
            // so the error ends the attempt; nothing continues in plaintext.
            let stream = tokio::time::timeout(limit, handshake)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, HandshakeTimedOut(limit)))??;
            let version = match stream.get_ref().1.protocol_version() {
                Some(rustls::ProtocolVersion::TLSv1_3) => Some("TLSv1.3"),
                Some(rustls::ProtocolVersion::TLSv1_2) => Some("TLSv1.2"),
                _ => None,
            };
            *self
                .negotiated
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = version;
            Ok(RustlsStream(Box::new(stream)))
        })
    }
}
pub(crate) struct RustlsStream(Box<tokio_rustls::client::TlsStream<Socket>>);
impl AsyncRead for RustlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_read(cx, buf)
    }
}
impl AsyncWrite for RustlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.get_mut().0).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_shutdown(cx)
    }
}
impl TlsStream for RustlsStream {
    /// No tls-server-end-point data: SCRAM authenticates without channel binding.
    /// Certificate verification (verify-full) is what authenticates the server.
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

/// The server accepted TLS but did not complete the handshake within this bound.
#[derive(Debug)]
struct HandshakeTimedOut(Duration);
impl std::fmt::Display for HandshakeTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the TLS handshake did not complete within {:?}", self.0)
    }
}
impl std::error::Error for HandshakeTimedOut {}

/// A specific, secret-free diagnostic for a failed TLS negotiation, or `None`
/// when the connect error is not about TLS (refused, authentication, timeout).
pub(crate) fn classify(error: &tokio_postgres::Error) -> Option<Diagnostic> {
    // tokio-postgres reports every TLS negotiation/handshake failure as its
    // Tls kind, displayed with this prefix and the cause as `source()`.
    if !error
        .to_string()
        .starts_with("error performing TLS handshake")
    {
        return None;
    }
    let cause = std::error::Error::source(error);
    if let Some(limit) = cause.and_then(handshake_timed_out) {
        return Some(Diagnostic::new(
            "DB_TLS",
            &format!(
                "The server did not complete the TLS handshake within {limit:?}; the connection was closed. No plaintext fallback was attempted."
            ),
        ));
    }
    let message = if cause.is_some_and(|c| c.to_string() == "server does not support TLS") {
        "The server does not accept TLS connections. No plaintext fallback was attempted."
    } else {
        match cause.and_then(certificate_error) {
            Some(CertificateError::NotValidForName)
            | Some(CertificateError::NotValidForNameContext { .. }) => {
                "The server certificate is not valid for the connection host name (sslmode=verify-full)."
            }
            Some(CertificateError::UnknownIssuer) | Some(CertificateError::BadSignature) => {
                "The server certificate is not issued by a trusted CA (host CA store or sslrootcert)."
            }
            Some(CertificateError::Expired)
            | Some(CertificateError::ExpiredContext { .. })
            | Some(CertificateError::NotValidYet)
            | Some(CertificateError::NotValidYetContext { .. }) => {
                "The server certificate is expired or not yet valid."
            }
            Some(_) => "The server certificate was rejected.",
            None => "The TLS handshake with the server failed.",
        }
    };
    Some(Diagnostic::new("DB_TLS", message))
}

/// The bound a handshake exceeded, when that is why it failed.
fn handshake_timed_out(cause: &(dyn std::error::Error + 'static)) -> Option<Duration> {
    let io = cause.downcast_ref::<io::Error>()?;
    io.get_ref()?
        .downcast_ref::<HandshakeTimedOut>()
        .map(|timed_out| timed_out.0)
}

/// tokio-rustls surfaces rustls failures as `io::Error` wrapping `rustls::Error`.
fn certificate_error(cause: &(dyn std::error::Error + 'static)) -> Option<CertificateError> {
    let io = cause.downcast_ref::<io::Error>()?;
    match io.get_ref()?.downcast_ref::<rustls::Error>()? {
        rustls::Error::InvalidCertificate(error) => Some(error.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a hang reaches this; a correct run takes milliseconds.
    const HANG_GUARD: Duration = Duration::from_secs(60);

    fn parse(secret: &str) -> Result<(Zeroizing<String>, Transport)> {
        split(secret)
    }
    fn code(secret: &str) -> String {
        parse(secret).unwrap_err().code
    }
    /// An absolute path on this platform, written as a connection secret
    /// would hold it (forward slashes: a backslash escapes in key/value form).
    fn absolute(name: &str) -> String {
        if cfg!(windows) {
            format!("C:/ntnt/{name}")
        } else {
            format!("/etc/ntnt/{name}")
        }
    }

    #[test]
    fn keyword_form_removes_tls_keys_and_keeps_other_bytes() {
        let (rest, tls) = parse(&format!(
            r"host=db.example.com port=6543 user=app dbname=app password='a b\'c' sslmode=verify-full sslrootcert='{}'",
            absolute("ca bundle.pem")
        ))
        .unwrap();
        assert_eq!(
            rest.as_str(),
            r"host=db.example.com port=6543 user=app dbname=app password='a b\'c'"
        );
        assert_eq!(tls.mode, Mode::VerifyFull);
        assert_eq!(
            tls.roots,
            Some(RootSource::File(absolute("ca bundle.pem").into()))
        );
        let config: tokio_postgres::Config = rest.parse().unwrap();
        assert_eq!(config.get_password(), Some(&b"a b'c"[..]));
        assert_eq!(config.get_ports(), &[6543]);
    }

    #[test]
    fn keyword_form_matches_tokio_postgres_spacing_and_escapes() {
        let (rest, tls) = parse(&format!(
            "  host = h  user=u\\ x dbname = d\tsslmode = verify-ca sslrootcert = '{}'   ",
            absolute("ca.pem")
        ))
        .unwrap();
        assert_eq!(tls.mode, Mode::VerifyCa);
        assert_eq!(tls.roots, Some(RootSource::File(absolute("ca.pem").into())));
        let config: tokio_postgres::Config = rest.parse().unwrap();
        assert_eq!(config.get_user(), Some("u x"));
        assert_eq!(config.get_dbname(), Some("d"));
        // Without sslmode in the remainder, tokio-postgres would default to
        // prefer; the caller always sets the mode explicitly.
        assert_eq!(
            config.get_ssl_mode(),
            tokio_postgres::config::SslMode::Prefer
        );
    }

    #[test]
    fn url_form_removes_tls_query_parameters() {
        let encoded = absolute("ca.pem").replace(':', "%3A").replace('/', "%2F");
        let (rest, tls) = parse(&format!(
            "postgresql://app:p%40ss@db.example.com:5432/app?application_name=x&sslmode=verify-full&sslrootcert={encoded}"
        ))
        .unwrap();
        assert_eq!(
            rest.as_str(),
            "postgresql://app:p%40ss@db.example.com:5432/app?application_name=x"
        );
        assert_eq!(tls.mode, Mode::VerifyFull);
        assert_eq!(tls.roots, Some(RootSource::File(absolute("ca.pem").into())));
        let (rest, tls) = parse("postgres://app@db/app?sslmode=require").unwrap();
        assert_eq!(rest.as_str(), "postgres://app@db/app");
        assert_eq!(tls.mode, Mode::Require);
        assert_eq!(tls.roots, None);
        let config: tokio_postgres::Config = rest.parse().unwrap();
        assert_eq!(config.get_user(), Some("app"));
    }

    #[test]
    fn modes_and_roots_follow_libpq_names_without_fallback() {
        assert_eq!(
            parse("host=h sslmode=disable").unwrap().1.mode,
            Mode::Disable
        );
        assert_eq!(
            parse("host=h sslmode=verify-full sslrootcert=system")
                .unwrap()
                .1
                .roots,
            Some(RootSource::System)
        );
        for rejected in [
            "host=h",
            "host=h sslmode=prefer",
            "host=h sslmode=allow",
            "host=h sslmode=verify",
            "host=h sslmode=disable sslmode=verify-full",
            "host=h sslmode=disable sslrootcert=/ca.pem",
            "host=h sslmode=require sslrootcert=/ca.pem",
            "host=h sslmode=verify-full sslrootcert=relative/ca.pem",
            "host=h sslmode=verify-full sslcert=/client.pem",
            "host=h sslmode=verify-full ssl_min_protocol_version=TLSv1.3",
            "postgres://u@h/d?sslmode=prefer",
            "host=h password='unterminated",
        ] {
            assert_eq!(code(rejected), "DB_CONNECTION_CONFIG", "{rejected}");
        }
    }

    #[test]
    fn runtime_split_defaults_only_a_missing_sslmode() {
        let local = |_: &str| Ok(Mode::Disable);
        let remote = |_: &str| Ok(Mode::VerifyFull);
        // The default sees the remainder without TLS keywords.
        let seen = std::cell::RefCell::new(String::new());
        let (rest, tls) = split_with_default("host=db user=u dbname=d", |rest| {
            seen.replace(rest.to_string());
            Ok(Mode::Disable)
        })
        .unwrap();
        assert_eq!(seen.into_inner(), "host=db user=u dbname=d");
        assert_eq!(
            (rest.as_str(), tls.mode),
            ("host=db user=u dbname=d", Mode::Disable)
        );
        let (_, tls) = split_with_default("host=db.example.com", remote).unwrap();
        assert_eq!(
            (tls.mode, tls.roots),
            (Mode::VerifyFull, Some(RootSource::System))
        );
        // An explicit sslmode is never replaced by the default.
        for (secret, mode) in [
            ("host=h sslmode=disable", Mode::Disable),
            ("host=h sslmode=require", Mode::Require),
            ("postgres://u@h/d?sslmode=verify-full", Mode::VerifyFull),
        ] {
            let (_, tls) = split_with_default(secret, |_| panic!("default consulted")).unwrap();
            assert_eq!(tls.mode, mode, "{secret}");
        }
        // A CA file or `system` selects verify-full even where the default is plaintext.
        let (_, tls) =
            split_with_default(&format!("host=h sslrootcert={}", absolute("ca.pem")), local)
                .unwrap();
        assert_eq!(
            (tls.mode, tls.roots),
            (
                Mode::VerifyFull,
                Some(RootSource::File(absolute("ca.pem").into()))
            )
        );
        let (_, tls) = split_with_default("host=h sslrootcert=system", local).unwrap();
        assert_eq!(
            (tls.mode, tls.roots),
            (Mode::VerifyFull, Some(RootSource::System))
        );
        // Downgradable and unknown modes stay rejected, as for ntnt db.
        for rejected in [
            "host=h sslmode=prefer",
            "host=h sslmode=allow",
            "host=h sslmode=verify",
            "host=h sslmode=verify-ca",
        ] {
            assert_eq!(
                split_with_default(rejected, local).unwrap_err().code,
                "DB_CONNECTION_CONFIG",
                "{rejected}"
            );
        }
        // ntnt db's own rule is unchanged: no default.
        assert!(split("host=h")
            .unwrap_err()
            .message
            .contains("Set sslmode explicitly"));
    }

    #[test]
    fn policy_names_mode_and_trust_source() {
        let policy = |secret: &str| split(secret).unwrap().1.policy();
        assert_eq!(policy("host=h sslmode=disable"), "disable");
        assert_eq!(policy("host=h sslmode=require"), "require");
        assert_eq!(
            policy("host=h sslmode=verify-full"),
            "verify-full roots=system"
        );
        assert_eq!(
            policy(&format!(
                "host=h sslmode=verify-ca sslrootcert={}",
                absolute("ca.pem")
            )),
            format!("verify-ca roots=file:{}", absolute("ca.pem"))
        );
    }

    #[test]
    fn verify_ca_requires_a_ca_file() {
        // verify-ca skips the host-name check, so host-store roots would accept
        // any certificate a public CA issued to anyone, for any name.
        for secret in [
            "host=h sslmode=verify-ca",
            "host=h sslmode=verify-ca sslrootcert=system",
            "postgres://u@h/d?sslmode=verify-ca",
            "postgres://u@h/d?sslrootcert=system&sslmode=verify-ca",
        ] {
            let error = parse(secret).unwrap_err();
            assert_eq!(error.code, "DB_CONNECTION_CONFIG", "{secret}");
            assert!(error.message.contains("sslrootcert="), "{secret}: {error}");
        }
        let (_, tls) = parse(&format!(
            "host=h sslmode=verify-ca sslrootcert={}",
            absolute("ca.pem")
        ))
        .unwrap();
        assert_eq!(tls.mode, Mode::VerifyCa);
        assert_eq!(tls.roots, Some(RootSource::File(absolute("ca.pem").into())));
        // verify-full keeps the host CA store as its default.
        assert_eq!(
            parse("host=h sslmode=verify-full").unwrap().1.roots,
            Some(RootSource::System)
        );
    }

    #[test]
    fn verify_ca_never_builds_a_chain_only_verifier_over_host_roots() {
        let transport = Transport {
            mode: Mode::VerifyCa,
            roots: Some(RootSource::System),
        };
        let error = connector(&transport).err().expect("rejected");
        assert_eq!(error.code, "DB_TLS", "{error}");
    }

    #[test]
    fn percent_decoding_matches_the_url_parser() {
        assert_eq!(percent_decode("%2Fetc%2fca.pem").unwrap(), "/etc/ca.pem");
        assert_eq!(percent_decode("100%").unwrap(), "100%");
        assert_eq!(percent_decode("%zz%4").unwrap(), "%zz%4");
        assert_eq!(code("postgres://u@h/d?sslmode=%FF"), "DB_CONNECTION_CONFIG");
    }

    #[test]
    fn diagnostics_never_echo_secret_contents() {
        for secret in [
            "host=h password=hunter2 sslmode=prefer",
            "host=h password=hunter2",
            "host=h password=hunter2 sslmode=verify-full sslrootcert=hunter2.pem",
            "host=h password=hunter2 sslmode=verify-ca",
            "host=h password=hunter2 sslmode=verify-ca sslrootcert=system",
            "postgres://u:hunter2@h/d?sslmode=bogus",
        ] {
            let error = parse(secret).unwrap_err();
            assert!(
                !error.message.contains("hunter2") && !error.next_action.contains("hunter2"),
                "{error}"
            );
        }
    }

    /// Like `database_runtime`: a blocked off-thread read cannot hold up shutdown.
    /// The clock is paused, and a paused clock does not advance while a blocking
    /// task runs, so `ROOTS_TIMEOUT` never fires on a slow machine: every result
    /// comes from the trust read itself.
    fn connector(transport: &Transport) -> Result<(Option<MakeRustls>, Value)> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        let result = runtime.block_on(transport.connector());
        runtime.shutdown_timeout(std::time::Duration::from_millis(100));
        result
    }

    #[test]
    fn url_form_accepts_a_trailing_ampersand_like_the_driver() {
        for (secret, mode) in [
            ("postgres://app@db/app?sslmode=disable&", Mode::Disable),
            (
                "postgres://app@db/app?application_name=x&sslmode=verify-full&",
                Mode::VerifyFull,
            ),
            (
                "postgres://app@db/app?sslmode=require&application_name=x&",
                Mode::Require,
            ),
        ] {
            let (rest, tls) = parse(secret).unwrap();
            assert_eq!(tls.mode, mode, "{secret}");
            let config: tokio_postgres::Config = rest.parse().unwrap();
            assert_eq!(config.get_user(), Some("app"), "{secret}");
        }
        // Only the trailing separator is accepted: an empty parameter in the
        // middle passes through unchanged and the driver rejects it, as before.
        let (rest, _) = parse("postgres://app@db/app?sslmode=disable&&application_name=x").unwrap();
        assert!(rest.parse::<tokio_postgres::Config>().is_err());
    }

    #[test]
    fn root_file_must_hold_a_usable_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        let junk = dir.path().join("junk.pem");
        std::fs::write(
            &junk,
            "-----BEGIN CERTIFICATE-----\nnot base64!\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        for path in [empty, junk, dir.path().join("missing.pem")] {
            let transport = Transport {
                mode: Mode::VerifyFull,
                roots: Some(RootSource::File(path.clone())),
            };
            let error = connector(&transport).err().expect("unusable roots");
            assert_eq!(error.code, "DB_CONNECTION_CONFIG", "{path:?}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_root_file_is_rejected_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("roots.pem");
        let c_path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path; mkfifo has no other preconditions.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let transport = Transport {
            mode: Mode::VerifyFull,
            roots: Some(RootSource::File(fifo)),
        };
        // No writer ever opens the FIFO, so a blocking open would never return,
        // and the paused clock keeps the trust-loading deadline from cutting it
        // off: only rejecting it as not a regular file produces a result. A
        // watchdog thread turns a blocking open into a failure at the hang guard.
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(connector(&transport).err());
        });
        let error = result
            .recv_timeout(HANG_GUARD)
            .expect("loading a FIFO as sslrootcert blocked")
            .expect("a FIFO is not a usable root file");
        assert_eq!(error.code, "DB_CONNECTION_CONFIG");
        assert!(error.message.contains("readable PEM file"), "{error}");
    }

    #[test]
    fn with_deadline_returns_the_result_or_abandons_a_stalled_worker() {
        assert_eq!(with_deadline(HANG_GUARD, || 7), Ok(7));
        // The work blocks until the test releases it, which happens only after
        // with_deadline has returned, so only the deadline can end the wait.
        // Run without one, the work would give up at its own hang guard and
        // return a sentinel, failing the assertion instead of hanging the test.
        let (release, released) = std::sync::mpsc::channel::<()>();
        let outcome = with_deadline(Duration::from_millis(10), move || {
            match released.recv_timeout(HANG_GUARD) {
                Ok(()) => "released",
                Err(_) => "hang guard",
            }
        });
        assert_eq!(outcome, Err(RecvTimeoutError::Timeout));
        let _ = release.send(());
        // A worker that ends without a result is reported, not waited for.
        let died = with_deadline(HANG_GUARD, || -> u8 { panic!("loader failed") });
        assert_eq!(died, Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn connector_blocking_passes_the_trust_read_result_through() {
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("ca.pem");
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        std::fs::write(&ca, params.self_signed(&key).unwrap().pem()).unwrap();
        let transport = |path: PathBuf| Transport {
            mode: Mode::VerifyFull,
            roots: Some(RootSource::File(path)),
        };
        assert!(transport(ca).connector_blocking().unwrap().is_some());
        let error = transport(dir.path().join("missing.pem"))
            .connector_blocking()
            .err()
            .expect("a missing root file is rejected");
        assert_eq!(error.code, "DB_CONNECTION_CONFIG", "{error}");
        assert!(error.message.contains("readable PEM file"), "{error}");
    }

    #[test]
    fn a_stalled_tls_handshake_fails_at_the_default_bound() {
        use std::io::Write;
        // The server accepts TLS ('S') and never answers the ClientHello; it
        // reports what it received once the client closes the connection.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (report, seen) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let mut request = [0u8; 8];
            client.read_exact(&mut request).unwrap();
            client.write_all(b"S").unwrap();
            let mut received = Vec::new();
            let _ = client.read_to_end(&mut received);
            let _ = report.send((request, received));
        });
        // The paused clock advances only when the client has nothing left to
        // do but wait, so the bound fires while the handshake waits for the
        // server, whatever the machine's speed; no other timer is armed. A
        // watchdog thread turns a missing bound into a failure at the hang guard.
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .start_paused(true)
                .build()
                .unwrap();
            let transport = Transport {
                mode: Mode::Require,
                roots: None,
            };
            let error = runtime.block_on(async {
                let (tls, _) = transport.connector().await.unwrap();
                let mut config = tokio_postgres::Config::new();
                config
                    .host("127.0.0.1")
                    .port(port)
                    .user("u")
                    .dbname("d")
                    .ssl_mode(tokio_postgres::config::SslMode::Require);
                config.connect(tls.unwrap()).await.err()
            });
            let _ = done.send(error.map(|error| classify(&error)));
        });
        let diagnostic = result
            .recv_timeout(HANG_GUARD)
            .expect("the stalled TLS handshake hung")
            .expect("a stalled handshake cannot connect")
            .expect("a TLS diagnostic");
        assert_eq!(diagnostic.code, "DB_TLS", "{diagnostic}");
        assert!(
            diagnostic
                .message
                .contains("did not complete the TLS handshake within 10s"),
            "{diagnostic}"
        );
        assert!(diagnostic.message.contains("No plaintext fallback"));
        // The client sent its ClientHello (a TLS handshake record) and then
        // closed the connection instead of continuing.
        let (request, received) = seen
            .recv_timeout(HANG_GUARD)
            .expect("the client kept the stalled connection open");
        assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47]);
        assert_eq!(received.first(), Some(&0x16), "{received:?}");
    }
}
