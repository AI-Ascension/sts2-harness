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
    raw.extend_from_slice(size_text.as_bytes());
    raw.extend_from_slice(b"\r\n");
    raw.extend_from_slice(body);
    raw.extend_from_slice(b"\r\n0\r\n\r\n");
    raw
}

#[test]
fn root_repro_2026_09_29_chunk_size_overflow_panics_instead_of_refusing() {
    // A declared chunk size of `usize::MAX` must be refused as an ordinary error. `catch_unwind`
    // is what makes this a regression test rather than a crash report: before the checked-arithmetic
    // fix the decode aborted the process here, in a function that only ever returns `Result`.
    let raw = frame("ffffffffffffffff", b"AB");
    let outcome = std::panic::catch_unwind(|| parse_body(&raw));
    assert!(
        outcome.is_ok(),
        "parse_body panicked on a hostile chunk size instead of refusing it"
    );
    assert!(
        parse_body(&raw).is_err(),
        "an oversized chunk size must be refused, not accepted or fatal"
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

    // A well-formed small body still decodes through the same checked path.
    assert_eq!(
        parse_body(&frame("2", b"AB")).expect("a valid chunked body"),
        b"AB"
    );

    // A chunk whose declared size exceeds the bytes actually present is a truncation: the
    // `rest[..size]` slice must never be reached with fewer than `size` bytes available.
    assert!(
        parse_body(&frame("4", b"AB")).is_err(),
        "a chunk shorter than its declared size must be refused"
    );

    // The bound itself and the value one below the wraparound extreme are honest oversizes, and
    // the extreme is refused too, so no size near `usize::MAX` can reach the slice.
    for size_text in [
        format!("{:x}", MAX_RESPONSE_BYTES),
        String::from("fffffffffffffffe"),
        String::from("ffffffffffffffff"),
    ] {
        assert!(
            parse_body(&frame(&size_text, b"AB")).is_err(),
            "an over-large chunk size must be refused rather than accepted or fatal"
        );
    }
}
