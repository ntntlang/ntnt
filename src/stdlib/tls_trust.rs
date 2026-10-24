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
        .get_or_init(|| load_host_trust(rustls_native_certs::load_native_certs))
        .as_ref()
        .map_err(Clone::clone)
}

fn load_host_trust(
    discover: impl FnOnce() -> rustls_native_certs::CertificateResult + std::panic::UnwindSafe,
) -> Result<HostTrust, String> {
    // Some native-store adapters can panic while decoding malformed OS entries.
    // Keep that dependency failure inside the cached Result; never retry it on
    // every TLS call or let a certificate-store problem unwind through the app.
    let native = std::panic::catch_unwind(discover).map_err(|_| {
        "no usable host CA certificates: native certificate discovery failed; repair the host CA store or set SSL_CERT_FILE / SSL_CERT_DIR, then restart NTNT".to_string()
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn native_discovery_panic_is_a_cached_setup_error() {
        let cache = OnceLock::new();
        let calls = AtomicUsize::new(0);
        for _ in 0..2 {
            let result = cache.get_or_init(|| {
                load_host_trust(|| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    panic!("simulated native-store metadata failure");
                })
            });
            match result {
                Err(error) => assert!(error.contains("native certificate discovery failed")),
                Ok(_) => panic!("failed native discovery must not produce trusted roots"),
            }
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "cache the failure instead of retrying discovery"
        );
    }
}
