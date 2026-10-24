//! Host CA roots shared by TLS inspection, HTTP probes, and SMTP.
//!
//! Discovery is local (not an online platform verifier). Cache the result once
//! per process; restart NTNT after changing the OS store or SSL_CERT_* overrides.
use rustls::pki_types::CertificateDer;
use rustls::RootCertStore;
use std::sync::{Arc, OnceLock};

pub(super) struct HostTrust {
    pub(super) roots: Arc<RootCertStore>,
    pub(super) certificates: Vec<CertificateDer<'static>>,
}

pub(super) fn host_trust() -> Result<&'static HostTrust, String> {
    static TRUST: OnceLock<Result<HostTrust, String>> = OnceLock::new();
    TRUST
        .get_or_init(load_host_trust)
        .as_ref()
        .map_err(Clone::clone)
}

fn load_host_trust() -> Result<HostTrust, String> {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    let mut certificates = Vec::new();
    // Native stores can contain individual malformed/unsupported entries. Keep
    // the usable subset, but never replace an empty store with bundled public CAs.
    for certificate in native.certs {
        if roots.add(certificate.clone()).is_ok() {
            certificates.push(certificate);
        }
    }
    if roots.is_empty() {
        return Err("no usable host CA certificates; install or update the OS CA store (ca-certificates in Linux/container images), or set SSL_CERT_FILE / SSL_CERT_DIR; restart NTNT after updating trust".to_string());
    }
    Ok(HostTrust {
        roots: Arc::new(roots),
        certificates,
    })
}
