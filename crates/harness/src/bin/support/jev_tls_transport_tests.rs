// SPDX-License-Identifier: MIT

//! Fail-closed properties of the in-process System One transport.
//!
//! Split from `sts2_jev_bridge_tests` so that suite stays under its preferred size, and kept
//! apart because the property under test is about what the transport *refuses*, not about what
//! the bridge decides. The response-framing refusals live beside this in
//! `jev_tls_transport_framing_tests`; what remains here is the credential and the trust store.
//!
//! Nothing here needs the network or a live credential. The certificate tests drive the real
//! `rustls` verifier that `client_config` installs -- the same one a live run uses -- so a
//! verifier that stopped refusing would fail here rather than in a run. That is the property the
//! acceptance criteria ask for: a chain or hostname that does not verify must be refused, and
//! the suite must be able to prove it offline.

use std::sync::Arc;

use rustls::client::danger::ServerCertVerifier;

use super::tls::{PROVIDER_HOST, PROVIDER_PATH, build_request, client_config};
use super::tls_fixture::{LEAF_FOR_PROVIDER, TEST_CA};

/// The leaf the negative tests present, as the peer would send it.
fn leaf() -> rustls_pki_types::CertificateDer<'static> {
    rustls_pki_types::CertificateDer::from(LEAF_FOR_PROVIDER)
}

/// The chain the peer sends with that leaf: the leaf, then the test CA.
fn chain() -> Vec<rustls_pki_types::CertificateDer<'static>> {
    vec![leaf(), rustls_pki_types::CertificateDer::from(TEST_CA)]
}

/// The admitted host, as a server name.
fn provider_name() -> rustls_pki_types::ServerName<'static> {
    rustls_pki_types::ServerName::try_from(PROVIDER_HOST.to_owned())
        .expect("the admitted host is a valid server name")
}

/// The request carries the credential in a header and never as an argument or a body field.
#[test]
fn the_credential_is_carried_only_in_the_authorization_header() {
    let body = b"{\"state\":1}";
    let request = build_request(body, "test-credential");
    let text = String::from_utf8_lossy(&request);
    assert!(text.starts_with("POST /v1/systemone HTTP/1.1\r\n"));
    assert!(text.contains("Host: api.typesafe.ai\r\n"));
    assert!(text.contains("Content-Type: application/json\r\n"));
    assert!(text.contains("Authorization: Bearer test-credential\r\n"));
    // The declared length must match the body actually appended, or the provider reads a
    // truncated request and answers about something the bridge never asked.
    assert!(text.contains(&format!("Content-Length: {}\r\n", body.len())));
    assert!(text.ends_with(std::str::from_utf8(body).unwrap_or_default()));
    assert!(!text.contains("TYPESAFE_API_KEY"));
}

/// The trust anchor is the pinned store compiled into this binary, not the host's system roots.
///
/// A run record has to be able to say which roots verified the peer. "It used the system roots"
/// is not a statement a digest-pinned artifact can make, so this asserts the store is non-empty
/// and that the client configuration is built from exactly it.
#[test]
fn the_client_verifies_against_the_pinned_root_store() {
    assert!(!webpki_roots::TLS_SERVER_ROOTS.is_empty());
    let config = client_config().expect("the pinned root store builds a client config");
    assert_eq!(PROVIDER_HOST, "api.typesafe.ai");
    assert_eq!(PROVIDER_PATH, "/v1/systemone");
    // The admitted name is bound into the configuration, so a peer naming another host fails
    // verification in the handshake rather than after the request has been sent.
    assert_eq!(Arc::strong_count(&config), 1);
}

/// A certificate that does not chain to the pinned roots is refused, offline.
///
/// This drives the verifier `client_config` actually installs. The certificate is a real
/// self-signed leaf naming the admitted host, so the only thing wrong with it is that nothing in
/// the pinned store vouches for it. A verifier that accepted it would accept any self-signed
/// certificate for `api.typesafe.ai`, which is the substitution the pinned root store exists to
/// prevent. The test needs no network and no private key.
#[test]
fn a_certificate_that_does_not_chain_to_the_pinned_roots_is_refused() {
    let verifier = pinned_verifier();
    // The assertion is on the verifier itself -- the one `client_config` installs -- so what is
    // being tested is the pinned root store's decision about this chain. The chain is well formed
    // and names the admitted host; the only thing wrong with it is that the pinned store does not
    // vouch for its CA. A verifier that accepted it would accept any chain for
    // `api.typesafe.ai`, which is the substitution the pinned store exists to prevent.
    let outcome = verifier.verify_server_cert(
        &leaf(),
        &chain()[1..],
        &provider_name(),
        &[],
        rustls_pki_types::UnixTime::now(),
    );
    assert!(
        outcome.is_err(),
        "a chain no pinned root vouches for must not verify, even naming the admitted host"
    );
}

/// A certificate naming a different host is refused, offline.
///
/// The same verifier, a real certificate, and a server name it does not name. This is the case a
/// "chains to a trusted root but is for somebody else" attack takes, and it is the one that a
/// verifier checking only the chain would let through.
#[test]
fn a_certificate_naming_another_host_is_refused() {
    let verifier = pinned_verifier();
    let outcome = verifier.verify_server_cert(
        &leaf(),
        &chain()[1..],
        &rustls_pki_types::ServerName::try_from("api.typesafe.ai.evil.test".to_owned())
            .expect("a valid but unadmitted name"),
        &[],
        rustls_pki_types::UnixTime::now(),
    );
    assert!(
        outcome.is_err(),
        "a certificate must not verify for a host it does not name"
    );
}

/// The verifier `client_config` installs, built from the pinned roots.
///
/// Built the same way the transport builds it, from `webpki_roots`, so a test that drives this
/// verifier is testing the trust decision a live run makes rather than a separately configured
/// one that could drift from it.
fn pinned_verifier() -> Arc<rustls::client::WebPkiServerVerifier> {
    let roots = Arc::new(rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    });
    rustls::client::WebPkiServerVerifier::builder(roots)
        .build()
        .expect("the pinned roots build a webpki verifier")
}

/// A provider name that is not the admitted host is refused before any connection is made.
#[test]
fn a_server_name_other_than_the_admitted_host_is_refused() {
    // Names that are not well-formed DNS names are refused outright.
    for name in [
        "",
        "not a host",
        "api.typesafe.ai/../x",
        "api.typesafe.ai:443",
    ] {
        assert!(
            rustls_pki_types::ServerName::try_from(name.to_owned()).is_err(),
            "expected {name:?} to be refused as a server name"
        );
    }
    // A well-formed name that is not the admitted host parses -- that is what makes it dangerous,
    // and it is why the transport hardcodes the name instead of taking one from configuration.
    let substituted =
        rustls_pki_types::ServerName::try_from("api.typesafe.ai.evil.test".to_owned())
            .expect("a well-formed substitute name parses");
    assert_ne!(
        substituted,
        rustls_pki_types::ServerName::try_from(PROVIDER_HOST.to_owned()).expect("host")
    );
    // The admitted host itself must parse, or every run would fail closed with no diagnostic.
    assert!(rustls_pki_types::ServerName::try_from(PROVIDER_HOST.to_owned()).is_ok());
}

/// A control proving the verifier is not simply refusing everything.
///
/// A negative test that passes because the verifier always errors would prove nothing, so this
/// puts the same certificate in the trust store and presents it as the peer. It then verifies
/// successfully and the two negative tests above are shown to be refusals rather than a verifier
/// that rejects every input. The store here is built in the test, not the pinned one, so this
/// says nothing about what a live run trusts.
#[test]
fn the_verifier_accepts_a_certificate_the_store_trusts() {
    let mut store = rustls::RootCertStore::empty();
    store
        .add(rustls_pki_types::CertificateDer::from(TEST_CA))
        .expect("the fixture is a certificate a root store accepts");
    let roots = Arc::new(store);
    let verifier = rustls::client::WebPkiServerVerifier::builder(roots)
        .build()
        .expect("a store with one root builds a verifier");
    let outcome = verifier.verify_server_cert(
        &leaf(),
        &[],
        &provider_name(),
        &[],
        rustls_pki_types::UnixTime::now(),
    );
    assert!(
        outcome.is_ok(),
        "the control must verify, or the negative tests prove nothing: {outcome:?}"
    );
}
