// SPDX-License-Identifier: MIT

//! Framing refusals of the in-process System One transport, asserted against literal bytes.
//!
//! Split out of `jev_tls_transport_tests` so both suites stay under their preferred size, and
//! kept apart because the property under test here is what the response *framer* refuses --
//! status, declared length, `chunked` shape, and the arithmetic that bounds a decoded body. None
//! of it needs the network, a live credential, or the `rustls` verifier, so every case is a
//! pure function of the bytes that arrived and each one asserts a named refusal rather than
//! merely that decoding stopped.

use super::tls::{MAX_HEADER_BYTES, MAX_RESPONSE_BYTES, parse_body};

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

/// A chunk size too large to be checked is a refusal, not a wrap.
///
/// A hex size is parsed into a `usize`, and a peer that declares `fffffffffffffffe` gets
/// `usize::MAX - 1` -- a value that is representable, so the parser accepts it, but far too large
/// to add to anything. Both guards that follow are additions: `decoded.len() + size` against the
/// body bound and `rest.len() < size + 2` against the bytes that actually arrived. Written as plain
/// additions both wrap to a small number, so a release build passes the body bound *and* the
/// truncation check and then attempts the out-of-bounds slice `&rest[..size]`, while a debug build
/// panics on the arithmetic. Neither is the intended refusal, and a provider answering on its own
/// behalf is what decides the number.
///
/// Both largest representable sizes are asserted with the specific refusal the body bound
/// produces, and both are reached with two bytes already decoded, because that is the only offset
/// at which the sum actually wraps (see the note on the frame below). Asserting the message rather
/// than merely "no panic" is the point: a decoder that clamped the size to the bound would also
/// avoid the panic, and that is the acceptance the test has to rule out.
#[test]
fn an_oversized_chunk_size_that_cannot_be_checked_is_refused() {
    for (size, expected) in [
        (
            "fffffffffffffffe",
            "the provider response exceeded its bound",
        ),
        (
            "ffffffffffffffff",
            "the provider response exceeded its bound",
        ),
    ] {
        // Two well-formed bytes are decoded first, so the run is two bytes into the body by the
        // time the huge size arrives. That offset is the whole point of the shape, and a smaller
        // one would make this test vacuous. `0 + fffffffffffffffe` is representable and `1 +
        // fffffffffffffffe` is exactly `usize::MAX`, so both are above the bound and correctly
        // refused by the *unfixed* code too -- a test using either would pass with the defect
        // still in place. `2 + fffffffffffffffe` is what wraps to `0`, so a plain addition
        // silently *passes* the body bound and the framing goes on to the truncation check,
        // which `3 < ffffffffffffffff + 2` also lets past.
        let raw = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nxy\r\n{size}\r\ny\r\n0\r\n\r\n"
        );
        assert_eq!(
            parse_body(raw.as_bytes())
                .expect_err("a chunk size no bound can hold is refused")
                .to_string(),
            expected,
            "expected the declared chunk size {size} to be refused as an oversized body"
        );
    }
    // The bound itself is unchanged for a size that is large but still checkable, so the refusal
    // above is the bound doing its job rather than a parser that has stopped accepting chunks.
    let over_bound = format!(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        MAX_RESPONSE_BYTES + 1
    );
    let mut oversized = over_bound.into_bytes();
    oversized.resize(oversized.len() + 16, b'x');
    oversized.extend_from_slice(b"\r\n0\r\n\r\n");
    assert_eq!(
        parse_body(&oversized)
            .expect_err("one byte over the bound is refused")
            .to_string(),
        "the provider response exceeded its bound"
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
