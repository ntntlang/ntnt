//! TLS transport for `ntnt db` PostgreSQL sessions.
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
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};
use tokio_postgres::Socket;
use zeroize::Zeroizing;

/// Largest accepted `sslrootcert` file. CA bundles are far smaller.
const MAX_ROOTS_BYTES: u64 = 1_048_576;

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
    config_error("Invalid connection secret; details suppressed.")
}

/// The remaining (still secret) connection string and the TLS keyword values.
type Split = (Zeroizing<String>, Vec<(String, String)>);

/// Split `sslmode`/`sslrootcert` out of a libpq key/value string or a
/// `postgres://` URL. Returns the remaining connection string for tokio-postgres
/// (still secret: it holds the password) and the requested transport. Errors
/// never echo connection-string contents.
pub(crate) fn split(secret: &str) -> Result<(Zeroizing<String>, Transport)> {
    let (rest, tls) = match ["postgresql://", "postgres://"]
        .iter()
        .find(|prefix| secret.starts_with(**prefix))
    {
        Some(prefix) => split_url(secret, prefix.len())?,
        None => split_keywords(secret)?,
    };
    Ok((rest, transport(tls)?))
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
    let query = &s[query_start + 1..];
    let mut kept: Vec<&str> = Vec::new();
    let mut tls = Vec::new();
    if !query.is_empty() {
        for segment in query.split('&') {
            let (key, value) = segment.split_once('=').ok_or_else(invalid_secret)?;
            let key = percent_decode(key)?;
            if is_tls_keyword(&key) {
                tls.push((key, percent_decode(value)?));
            } else {
                kept.push(segment);
            }
        }
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

/// Interpret the TLS keywords. `sslmode` must be explicit; `prefer`/`allow`
/// are rejected because they can fall back to plaintext.
fn transport(tls: Vec<(String, String)>) -> Result<Transport> {
    let mut mode: Option<String> = None;
    let mut roots: Option<String> = None;
    for (key, value) in tls {
        let slot = match key.as_str() {
            "sslmode" => &mut mode,
            "sslrootcert" => &mut roots,
            _ => {
                return Err(config_error(
                    "Unsupported TLS keyword in the connection secret. ntnt db supports sslmode and sslrootcert; client certificates, CRLs and protocol-version keywords are not supported.",
                ))
            }
        };
        if slot.replace(value).is_some() {
            return Err(config_error(
                "sslmode or sslrootcert appears more than once in the connection secret.",
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
        None => {
            return Err(config_error(
                "Set sslmode explicitly: verify-full (recommended), verify-ca, require, or disable on a trusted local transport.",
            ))
        }
    };
    let roots = match (mode, roots) {
        (Mode::VerifyCa | Mode::VerifyFull, None) => Some(RootSource::System),
        (Mode::VerifyCa | Mode::VerifyFull, Some(value)) if value == "system" => {
            Some(RootSource::System)
        }
        (Mode::VerifyCa | Mode::VerifyFull, Some(path)) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(config_error(
                    "sslrootcert must be an absolute path to a PEM file of CA certificates, or `system`.",
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

impl Transport {
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }
    /// Load trust anchors and build the connector, or `None` for plaintext.
    /// Also returns the transport description that identity reports bind.
    pub(crate) fn connector(&self) -> Result<(Option<MakeRustls>, Value)> {
        let mut description = json!({"sslmode": self.mode.as_str(), "roots": null});
        if self.mode == Mode::Disable {
            return Ok((None, description));
        }
        let roots = match &self.roots {
            None => None,
            Some(RootSource::System) => {
                description["roots"] = json!("system");
                let trust = crate::stdlib::tls_trust::host_trust()
                    .map_err(|message| Diagnostic::new("DB_TLS", &message))?;
                Some(trust.roots.clone())
            }
            Some(RootSource::File(path)) => {
                let (store, sha256) = file_roots(path)?;
                description["roots"] = json!("file");
                description["roots_sha256"] = json!(sha256);
                Some(Arc::new(store))
            }
        };
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|_| Diagnostic::new("DB_TLS", "TLS configuration failed."))?;
        let mut config = match (self.mode, roots) {
            (Mode::VerifyFull, Some(roots)) => {
                builder.with_root_certificates(roots).with_no_client_auth()
            }
            (Mode::VerifyCa, Some(roots)) => builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(ChainOnly { roots, provider }))
                .with_no_client_auth(),
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
            }),
            description,
        ))
    }
}

/// Read an `sslrootcert` PEM file: bounded, regular file, at least one usable CA.
fn file_roots(path: &std::path::Path) -> Result<(RootCertStore, String)> {
    let unreadable = || {
        config_error(
            "sslrootcert must name a readable PEM file of CA certificates (at most 1 MiB).",
        )
    };
    let file = std::fs::File::open(path).map_err(|_| unreadable())?;
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
        })
    }
}
pub(crate) struct RustlsConnect {
    config: Arc<rustls::ClientConfig>,
    name: ServerName<'static>,
}
impl TlsConnect<Socket> for RustlsConnect {
    type Stream = RustlsStream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<RustlsStream>> + Send>>;
    fn connect(self, stream: Socket) -> Self::Future {
        Box::pin(async move {
            tokio_rustls::TlsConnector::from(self.config)
                .connect(self.name, stream)
                .await
                .map(|stream| RustlsStream(Box::new(stream)))
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
        let (rest, tls) =
            parse("  host = h  user=u\\ x dbname = d\tsslmode = verify-ca   ").unwrap();
        assert_eq!(tls.mode, Mode::VerifyCa);
        assert_eq!(tls.roots, Some(RootSource::System));
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
            "postgres://u:hunter2@h/d?sslmode=bogus",
        ] {
            let error = parse(secret).unwrap_err();
            assert!(
                !error.message.contains("hunter2") && !error.next_action.contains("hunter2"),
                "{error}"
            );
        }
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
            let error = transport.connector().err().expect("unusable roots");
            assert_eq!(error.code, "DB_CONNECTION_CONFIG", "{path:?}: {error}");
        }
    }
}
