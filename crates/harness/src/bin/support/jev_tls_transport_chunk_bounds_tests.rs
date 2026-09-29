// SPDX-License-Identifier: MIT

//! Reproductions for the chunked-body size guards, split out so the main transport suite stays
//! under its preferred file size.
//!
//! The two guards in `decode_chunked` both take a provider-chosen `size`. At head `ba2b35d` they
//! used unchecked addition: a declared size of `usize::MAX` wrapped `decoded.len() + size` to a
//! small value, skipping the response bound, and wrapped `size + 2` to `1`, skipping the
//! truncation guard, so `rest[..size]` panicked instead of refusing the frame.
//!
//! **The accumulator is what makes this observable.** `decoded.len() + size` only wraps when
//! `decoded.len()` is nonzero, so a frame whose *first* chunk is the hostile one is refused by the
//! bound guard for an entirely honest reason — the declared size really is far above the 128 KiB
//! response bound — and a test built on that shape passes against the defective code. Every
//! hostile frame here therefore leads with a small valid chunk, so `decoded.len() == 2` when the
//! hostile size is read and both additions genuinely wrap.

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
fn an_oversized_chunk_size_that_cannot_be_checked_is_refused() {
    // `decoded.len()` is 2 when the hostile size is read, so `2 + 0xffffffffffffffff` wraps to 1,
    // which is *not* above `MAX_RESPONSE_BYTES`, and `size + 2` wraps to 0, which is *not* above
    // the bytes that remain. Both guards are skipped and the decode reaches `rest[..size]`, where
    // an unchecked build panics. `catch_unwind` is what makes this a regression test rather than a
    // crash report: `parse_body` only ever returns `Result`, so an unwind here is a contract
    // violation, not an expected outcome.
    let mut raw = HEAD.to_vec();
    raw.extend_from_slice(b"2\r\nxy\r\n");
    raw.extend_from_slice(b"fffffffffffffffe\r\n");
    raw.extend_from_slice(b"y\r\n0\r\n\r\n");
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

    // The bound itself is an honest oversize: no arithmetic wraps, and any correct implementation
    // rejects it. It is kept because it is a real boundary, not because it exercises the checked
    // addition.
    assert!(
        parse_body(&frame(&format!("{:x}", MAX_RESPONSE_BYTES), b"AB")).is_err(),
        "a body above the response bound must be refused rather than accepted or fatal"
    );

    // The two near-`usize::MAX` values are only refused *because* of the checked arithmetic, so
    // they are asserted behind a leading chunk. With `decoded.len() == 2` the bound sum wraps to 1
    // and the truncation sum wraps to 0, so the unchecked guards let the frame through to the
    // slice; against the fixed code both are ordinary refusals. Asserting them without the leading
    // chunk would pass on the defective code, which is exactly the trap this file documents.
    for (size_text, label) in [
        (String::from("fffffffffffffffe"), "one below usize::MAX"),
        (String::from("ffffffffffffffff"), "usize::MAX"),
    ] {
        let mut raw = HEAD.to_vec();
        raw.extend_from_slice(b"2\r\nxy\r\n");
        raw.extend_from_slice(size_text.as_bytes());
        raw.extend_from_slice(b"\r\ny\r\n0\r\n\r\n");
        let outcome = std::panic::catch_unwind(|| parse_body(&raw));
        assert!(
            outcome.is_ok(),
            "parse_body unwound on a declared size of {label} instead of refusing it"
        );
        assert!(
            parse_body(&raw).is_err(),
            "a declared size of {label} must be refused, not accepted or fatal"
        );
    }
}
