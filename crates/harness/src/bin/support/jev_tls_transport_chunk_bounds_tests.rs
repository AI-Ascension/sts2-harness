// SPDX-License-Identifier: MIT

//! Reproductions for the chunked-body size guards, split out so the main transport suite stays
//! under its preferred file size.
//!
//! The two guards in `decode_chunked` both take a provider-chosen `size`. At head `ba2b35d` they
//! used unchecked addition: a declared size of `usize::MAX` wrapped `decoded.len() + size` to a
//! small value, skipping the response bound, and wrapped `size + 2` to `1`, skipping the
//! truncation guard, so `rest[..size]` panicked instead of refusing the frame.

use super::tls::{MAX_RESPONSE_BYTES, parse_body};

const HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";

fn frame(size_text: &str, body: &[u8]) -> Vec<u8> {
    let mut raw = HEAD.to_vec();
    // A two-byte chunk first, so the decoder is two bytes into the body when the declared size
    // arrives. That offset is the whole point: `0 + fffffffffffffffe` is representable and
    // `1 + fffffffffffffffe` is exactly `usize::MAX`, so both are above the bound and are refused
    // even by the unfixed code, while `2 + fffffffffffffffe` wraps to 0 and `2 + ffffffffffffffff`
    // wraps to 1, so an unfixed decoder walks straight into the out-of-bounds slice. A frame that
    // declared the huge size first would never reach the wrap and would pass with the defect in
    // place, so the prefix is load-bearing rather than decoration.
    raw.extend_from_slice(b"2\r\n");
    raw.extend_from_slice(body);
    raw.extend_from_slice(b"\r\n");
    raw.extend_from_slice(size_text.as_bytes());
    raw.extend_from_slice(b"\r\n");
    raw.extend_from_slice(b"\r\n0\r\n\r\n");
    raw
}

#[test]
fn root_repro_2026_09_29_chunk_size_overflow_panics_instead_of_refusing() {
    // `catch_unwind` is what makes this a regression test rather than a crash report: before the
    // checked-arithmetic fix the decode aborted the process here, in a function that only ever
    // returns `Result`.
    let raw = frame("ffffffffffffffff", b"AB");
    let outcome = std::panic::catch_unwind(|| parse_body(&raw));
    assert!(
        outcome.is_ok(),
        "parse_body panicked on a hostile chunk size instead of refusing it"
    );
    // Asserting the message rather than merely "no panic" is the point: a decoder that clamped the
    // size to the bound would also avoid the panic, and that acceptance has to be ruled out.
    assert!(
        parse_body(&raw)
            .expect_err("a chunk size no bound can hold is refused")
            .to_string()
            == "the provider response exceeded its bound",
        "a declared chunk size of usize::MAX must be refused as an oversized body, not clamped"
    );
}

/// The `usize::MAX` case above is the wraparound extreme. These cover the ordinary boundaries the
/// checked arithmetic still has to honour: an honest oversize is refused, a valid small body still
/// decodes, and a chunk one byte short of its declared size is still a truncation.
#[test]
fn root_repro_2026_09_29_chunk_size_boundaries_still_refuse_and_accept_correctly() {
    // An honest size above the response bound is refused rather than decoded.
    let honest_oversize = frame(&format!("{:x}", MAX_RESPONSE_BYTES + 1), b"{}");
    assert!(
        parse_body(&honest_oversize).is_err(),
        "a body above the response bound must be refused"
    );

    // The same refusal when the declared size *is* honoured by the bytes: the frame is cut short,
    // so the decoder has to notice the body stops rather than reading past what the peer sent.
    let mut truncated = HEAD.to_vec();
    truncated.extend_from_slice(format!("{:x}\r\n", MAX_RESPONSE_BYTES + 1).as_bytes());
    truncated.resize(truncated.len() + 16, b'x');
    truncated.extend_from_slice(b"\r\n0\r\n\r\n");
    assert!(
        parse_body(&truncated)
            .expect_err("a chunk cut short of its declared size is refused")
            .to_string()
            == "the provider response exceeded its bound",
        "a truncated chunk above the bound is refused as an oversized body"
    );

    // A well-formed small body still decodes through the same checked path.
    let mut small = HEAD.to_vec();
    small.extend_from_slice(b"2\r\nAB\r\n0\r\n\r\n");
    assert_eq!(parse_body(&small).expect("a valid chunked body"), b"AB");

    // A chunk whose declared size exceeds the bytes actually present is a truncation: the
    // `rest[..size]` slice must never be reached with fewer than `size` bytes available.
    let mut short = HEAD.to_vec();
    short.extend_from_slice(b"4\r\nAB\r\n0\r\n\r\n");
    assert!(
        parse_body(&short).is_err(),
        "a chunk shorter than its declared size must be refused"
    );

    // A size exactly at the bound is not over the bound, so a frame that stops short of it is a
    // truncation, not an oversized body. Both refusals have to stay distinct.
    let mut at_bound = HEAD.to_vec();
    at_bound.extend_from_slice(format!("{:x}\r\n", MAX_RESPONSE_BYTES).as_bytes());
    at_bound.extend_from_slice(b"AB\r\n0\r\n\r\n");
    assert_eq!(
        parse_body(&at_bound)
            .expect_err("a chunk cut short of the bound is refused")
            .to_string(),
        "the provider response was truncated inside a chunk",
        "a size exactly at the bound is bounded, so only a short frame refuses it"
    );

    // One below the wraparound extreme, and the extreme itself, are both refused as an oversized
    // body rather than clamped, so no size near `usize::MAX` can reach the slice. The size exactly
    // at the bound is *not* in this set: it is not over the bound, and a frame that stops short of
    // it is a truncation, which is the other refusal.
    for size_text in ["fffffffffffffffe", "ffffffffffffffff"] {
        assert!(
            parse_body(&frame(size_text, b"AB"))
                .expect_err("a chunk size above the bound is refused")
                .to_string()
                == "the provider response exceeded its bound",
            "an over-large chunk size must be refused as an oversized body, not clamped"
        );
    }
}
