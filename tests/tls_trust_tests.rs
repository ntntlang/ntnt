//! Offline regressions for the October 2026 eMudhra intermediate-key compromise.

use rustls::client::danger::ServerCertVerifier;
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{pem::PemObject, CertificateDer, ServerName, UnixTime};
use rustls::RootCertStore;
use std::sync::Arc;
use std::time::Duration;

macro_rules! chains {
    ($($host:literal),+ $(,)?) => {
        &[$(($host, include_bytes!(concat!("fixtures/emudhra/", $host, ".pem")).as_slice())),+]
    };
}

const CHAINS: &[(&str, &[u8])] = chains![
    "dv-g3b-emudhra.we-have-a-problem.com",
    "dv-g3c-emudhra.we-have-a-problem.com",
    "ov-g3b-emudhra.we-have-a-problem.com",
    "ov-g3c-emudhra.we-have-a-problem.com",
    "ev-g3b-emudhra.we-have-a-problem.com",
    "ev-g3c-emudhra.we-have-a-problem.com",
    "dv-g3b-aaa-emudhra.we-have-a-problem.com",
    "dv-g3c-aaa-emudhra.we-have-a-problem.com",
    "ov-g3b-aaa-emudhra.we-have-a-problem.com",
    "ov-g3c-aaa-emudhra.we-have-a-problem.com",
    "ev-g3b-aaa-emudhra.we-have-a-problem.com",
    "ev-g3c-aaa-emudhra.we-have-a-problem.com",
];

fn certificates(pem: &[u8]) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(pem)
        .map(|cert| cert.expect("public certificate fixture must parse"))
        .collect()
}

fn verifier(roots: RootCertStore) -> Arc<WebPkiServerVerifier> {
    WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap()
}

fn verify(
    verifier: &WebPkiServerVerifier,
    host: &str,
    chain: &[CertificateDer<'static>],
) -> Result<(), rustls::Error> {
    // All fixtures are in-date here. Wall-clock expiry must not make a
    // vulnerable dependency look secure after these leaves expire.
    let now = UnixTime::since_unix_epoch(Duration::from_secs(1_792_497_600));
    verifier
        .verify_server_cert(
            &chain[0],
            &chain[1..],
            &ServerName::try_from(host).unwrap(),
            &[],
            now,
        )
        .map(|_| ())
}

#[test]
fn bundled_certificate_roots_reject_all_emudhra_test_chains() {
    // Same DER root bundle used by tls_info() and probe_fetch().
    let mut roots = RootCertStore::empty();
    let (added, ignored) =
        roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
    assert!(added > 0);
    assert_eq!(ignored, 0);
    assert_all_chains_rejected(&verifier(roots));
}

#[test]
fn bundled_smtp_trust_anchors_reject_all_emudhra_test_chains() {
    // Same trust-anchor bundle used transitively by lettre's SMTP TLS.
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    assert!(!roots.is_empty());
    assert_all_chains_rejected(&verifier(roots));
}

fn assert_all_chains_rejected(verifier: &WebPkiServerVerifier) {
    assert_eq!(CHAINS.len(), 12);
    for (host, pem) in CHAINS {
        let chain = certificates(pem);
        assert_eq!(chain.len(), if host.contains("-aaa-") { 2 } else { 3 });
        let result = verify(verifier, host, &chain);
        assert!(result.is_err(), "compromised chain still trusted: {host}");
    }
}

#[test]
fn compromised_intermediate_keys_are_blocked_even_with_explicitly_trusted_roots() {
    // Root removal alone must not be the only protection: exercise the
    // upstream SPKI block with both original and AAA cross-signed paths.
    let mut roots = RootCertStore::empty();
    for pem in [
        include_bytes!("fixtures/emudhra/emsign-ecc-g3-root.pem").as_slice(),
        include_bytes!("fixtures/emudhra/aaa-root.pem").as_slice(),
    ] {
        for cert in certificates(pem) {
            roots.add(cert).unwrap();
        }
    }
    let verifier = verifier(roots);
    for (host, pem) in CHAINS {
        let result = verify(&verifier, host, &certificates(pem));
        let error = result.expect_err(&format!("compromised key still trusted: {host}"));
        assert!(
            format!("{error:?}").contains("CertDistrusted"),
            "{host}: expected explicit key distrust, not an unrelated failure: {error:?}"
        );
    }
}

#[test]
fn ordinary_valid_certificate_chain_remains_trusted() {
    let mut ca_params = rcgen::CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["control.example".into()])
        .unwrap()
        .signed_by(&leaf_key, &ca, &ca_key)
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    verify(&verifier(roots), "control.example", &[leaf.der().clone()]).unwrap();
}
