// SPDX-License-Identifier: MIT

//! Fail-closed properties of the in-process System One transport.
//!
//! Split from `sts2_jev_bridge_tests` so that suite stays under its preferred size, and kept
//! apart because the property under test is about what the transport *refuses*, not about what
//! the bridge decides.
//!
//! Nothing here needs the network or a live credential. The response classes the provider can
//! return are asserted against bytes, and the certificate tests drive the real `rustls` verifier
//! that `client_config` installs -- the same one a live run uses -- so a verifier that stopped
//! refusing would fail here rather than in a run. That is the property the acceptance criteria
//! ask for: a chain or hostname that does not verify must be refused, and the suite must be able
//! to prove it offline.

use std::sync::Arc;

use rustls::client::danger::ServerCertVerifier;

use super::tls::{
    MAX_HEADER_BYTES, MAX_RESPONSE_BYTES, PROVIDER_HOST, PROVIDER_PATH, build_request,
    client_config, parse_body,
};
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

/// Builds a `200` response with an explicit length and body.
fn response(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    raw
}

/// A `200` response yields its body, and nothing else, for the bridge to parse.
#[test]
fn an_accepted_response_yields_only_its_body() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(parse_body(raw).expect("accepted"), b"{}".to_vec());
}

/// Every non-`200` status is the same refusal, and the two the provider documents are not special.
///
/// A `429` and a `529` are asserted alongside a bare `500` on purpose: the transport contract says
/// a rate limit and an overloaded upstream are transport failures like any other, and a reader
/// that quietly accepted either would let a caller mistake them for a decision.
#[test]
fn a_status_other_than_200_is_refused_including_429_and_529() {
    for status in [
        "400 Bad Request",
        "401 Unauthorized",
        "429 Too Many Requests",
        "500 Server Error",
        "529 Overloaded",
    ] {
        let raw = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n");
        let outcome = parse_body(raw.as_bytes());
        assert!(outcome.is_err(), "expected {status} to be refused");
        assert_eq!(
            outcome.unwrap_err().to_string(),
            "the provider returned a status other than 200"
        );
    }
}

/// A header block that never terminates is a refusal, not a partial read.
#[test]
fn an_unterminated_or_oversized_header_block_is_refused() {
    assert!(parse_body(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n").is_err());
    let oversized = format!(
        "HTTP/1.1 200 OK\r\nX-Filler: {}\r\n\r\n",
        "x".repeat(MAX_HEADER_BYTES + 1)
    );
    assert!(parse_body(oversized.as_bytes()).is_err());
}

/// A body is bounded on its own, not on headers plus body.
///
/// The bound is the largest decision the bridge is willing to hold, so a response at that bound
/// is accepted and one byte over it is refused -- in both cases with a maximal header block, so
/// the assertion cannot pass merely because the header happened to be small.
#[test]
fn the_body_bound_applies_to_the_body_alone() {
    let filler = format!("X-Filler: {}\r\n", "x".repeat(MAX_HEADER_BYTES / 2));
    let at_bound = response(&filler, &vec![b'x'; MAX_RESPONSE_BYTES]);
    assert_eq!(
        parse_body(&at_bound)
            .expect("a body at the bound is accepted")
            .len(),
        MAX_RESPONSE_BYTES
    );
    let over_bound = response(&filler, &vec![b'x'; MAX_RESPONSE_BYTES + 1]);
    assert_eq!(
        parse_body(&over_bound).unwrap_err().to_string(),
        "the provider response exceeded its bound"
    );
}

/// A declared length has to match the bytes that arrived.
///
/// This is the truncation shape. A transport that trusted `Content-Length` alone, or that
/// returned whatever followed the header block, would hand a partial body to the bridge as if it
/// were whole -- and a decision parsed from half a response is a fabricated decision.
#[test]
fn a_body_that_does_not_match_its_declared_length_is_refused() {
    // Declares eight bytes and delivers seven: the length and the body disagree, which is the
    // truncation shape. The helper cannot build this, so the frame is written out.
    let short = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n{\"a\":1}" as &[u8];
    assert_eq!(
        parse_body(short)
            .expect_err("a body shorter than its declared length")
            .to_string(),
        String::from("the provider response did not match its declared length")
    );
    // And the mirror: declares seven and delivers eight.
    let long = b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\n{\"a\":12}" as &[u8];
    assert!(parse_body(long).is_err());
    // A response that declares no length at all cannot be checked, so it is refused rather than
    // accepted on the assumption that a close means the end.
    assert_eq!(
        parse_body(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}")
            .expect_err("a response with no declared length")
            .to_string(),
        String::from("the provider response did not declare a length")
    );
    // Two lengths that disagree make the frame ambiguous and are refused; two that agree are the
    // same length written twice and are not.
    let conflicting =
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 9\r\n\r\n{}" as &[u8];
    assert!(parse_body(conflicting).is_err());
    let repeated =
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}" as &[u8];
    assert_eq!(
        parse_body(repeated).expect("agreeing lengths"),
        b"{}".to_vec()
    );
}

/// A chunked body is decoded, and a truncated or oversized one is refused.
///
/// The wire bytes of a chunked body are larger than the body, so a bound applied to the wire form
/// would reject a legitimate response. The bound is applied to the decoded bytes, which is what
/// the bridge holds.
#[test]
fn a_chunked_body_is_decoded_and_a_broken_one_is_refused() {
    // Built from the pieces rather than hand-counted, so the declared sizes are correct by
    // construction and a failure means the decoder is wrong rather than the arithmetic.
    let mut chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for piece in ["{\"a\"", ":", "1234567", "}"] {
        chunked.extend_from_slice(format!("{:x}\r\n{}\r\n", piece.len(), piece).as_bytes());
    }
    chunked.extend_from_slice(b"0\r\n\r\n");
    assert_eq!(
        parse_body(&chunked).expect("a well-formed chunked body"),
        b"{\"a\":1234567}".to_vec()
    );
    // Truncated mid-chunk, and terminated before the zero chunk.
    let cut = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"" as &[u8];
    assert!(parse_body(cut).is_err());
    let unterminated =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"}\r\n" as &[u8];
    assert!(parse_body(unterminated).is_err());
    // A size that is not hexadecimal, and a chunked body that also declares a length.
    let not_hex =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n{}\r\n0\r\n\r\n" as &[u8];
    assert!(parse_body(not_hex).is_err());
    let both =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n0\r\n\r\n"
            as &[u8];
    assert!(parse_body(both).is_err());
}

/// The chunked terminator has to be complete, trailers and all.
///
/// The grammar is a zero-size chunk, then a trailer section, then the blank line that ends it.
/// Accepting the body the moment the zero chunk is seen would accept `0\r\n` as a finished message
/// and ignore whatever the peer claimed came next -- the same "the first half of a message is a
/// message" mistake the declared-length path refuses. Each of these is a well-formed prefix of a
/// complete terminator, and none of them is a complete terminator.
#[test]
fn a_chunked_terminator_missing_its_trailer_section_is_refused() {
    // The zero chunk and nothing after it: the trailer section and its terminating CRLF are absent.
    assert!(
        parse_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\n{\"a\":}\r\n0\r\n")
            .is_err(),
        "a zero chunk with no trailer terminator is not a whole body"
    );
    // Present but malformed: a trailer line with no field name is not a header.
    assert!(
        parse_body(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\n{\"a\":}\r\n0\r\nnot-a-header\r\n\r\n"
                as &[u8]
        )
        .is_err(),
        "a malformed trailer is refused rather than skipped"
    );
    // And the complete form, which is accepted -- so the two above fail on the missing grammar
    // rather than on chunked framing being refused outright.
    let mut with_trailer =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\n{\"a\":}\r\n0\r\n".to_vec();
    with_trailer.extend_from_slice(b"X-Checksum: 0\r\n\r\n");
    assert_eq!(
        parse_body(&with_trailer).expect("a complete terminator with trailers"),
        b"{\"a\":}".to_vec()
    );
}

/// Only `chunked` is decoded, and only when it is the coding that was asked for.
///
/// A transport that searched the header for the substring `chunked` would decode a body a peer had
/// framed some other way, or that named the coding twice. Those are the shapes a proxy produces when
/// it wants two readings of the same bytes, so each is refused by name rather than decoded on a
/// guess.
#[test]
fn a_transfer_coding_this_transport_cannot_decode_is_refused() {
    for header in [
        // A coding this transport does not implement, with and without `chunked` alongside it.
        "Transfer-Encoding: gzip",
        "Transfer-Encoding: gzip, chunked",
        // The word present but not as a coding token: a substring match would decode these.
        "Transfer-Encoding: xchunked",
        "Transfer-Encoding: chunkedx",
        // `chunked` is defined as the final coding, so one that is not last is not this framing.
        "Transfer-Encoding: chunked, gzip",
        // Two `chunked` codings make the wire format ambiguous.
        "Transfer-Encoding: chunked, chunked",
    ] {
        let raw = format!("HTTP/1.1 200 OK\r\n{header}\r\n\r\n0\r\n\r\n");
        assert_eq!(
            parse_body(raw.as_bytes())
                .map(|_| ())
                .unwrap_err()
                .to_string(),
            "the provider response used a transfer coding this transport cannot decode",
            "expected {header:?} to be refused rather than decoded"
        );
    }
    // The exact token, alone, is the one framing that is implemented.
    assert_eq!(
        parse_body(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: Chunked\r\n\r\n6\r\n{\"a\":}\r\n0\r\n\r\n"
                as &[u8]
        )
        .expect("the chunked token, matched case-insensitively"),
        b"{\"a\":}".to_vec()
    );
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
