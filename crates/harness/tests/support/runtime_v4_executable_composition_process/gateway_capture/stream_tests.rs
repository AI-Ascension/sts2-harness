// SPDX-License-Identifier: MIT

//! Unit tests for the served gateway capture's two retention ceilings. Refs sts2-harness#559.
//!
//! #559's acceptance criterion 1 requires a stated *per-stream and total* ceiling. The
//! end-to-end test in `served_gateway_capture_drain.rs` can only exercise the per-stream axis at
//! all: proving the shared total needs a capture that is larger than a test is willing to spawn,
//! and a stub that wrote 8 MiB to reach it would test the writer's speed rather than the bound.
//! So the bounds are driven directly here, exactly as `gateway_evidence_tests.rs` drives the
//! evidence writer's 4 MiB bound.
//!
//! The totals are restated as literals rather than imported, so this file cannot agree with the
//! implementation by construction: moving a constant has to move the number here, or these
//! assertions fail. That is the point — a bound that silently drifts is worse than no bound.

use super::{Budget, MAX_CAPTURE_BYTES, MAX_TOTAL_CAPTURE_BYTES, Stream};

/// The per-stream ceiling, restated so this file cannot inherit a moved constant.
const PER_STREAM: usize = 4 * 1024 * 1024;

/// The shared total ceiling, restated for the same reason.
const TOTAL: usize = 8 * 1024 * 1024;

/// The notice a clipped stream ends with.
const NOTICE: &str = "{\"event\":\"gateway_output_truncated\"}";

/// A stream of `size` bytes whose content is position-dependent, so a cut is detectable as
/// content rather than only as a length.
fn bytes_of(size: usize) -> Vec<u8> {
    (0..size).map(|index| b'a' + (index % 26) as u8).collect()
}

/// A `Stream` whose reader is an owned, empty buffer.
///
/// These tests drive [`Stream::retain`] directly — the retention rule and its two ceilings —
/// and `retain` never reads from the pipe, so the reader only has to exist and satisfy `Read`
/// for the `Stream`'s bound. An owned `Vec` reader keeps that true without a temporary lifetime
/// the borrow checker would (correctly) reject.
fn empty_stream(label: &'static str) -> Stream<std::io::Empty> {
    Stream::new(std::io::empty(), label)
}

/// The two ceilings must be the values #559 states. If either moves, this fails and the
/// documentation, the issue, and the test all have to move with it.
#[test]
fn the_two_ceilings_are_the_documented_ones() {
    assert_eq!(
        MAX_CAPTURE_BYTES, PER_STREAM,
        "the per-stream ceiling moved"
    );
    assert_eq!(
        MAX_TOTAL_CAPTURE_BYTES, TOTAL,
        "the shared total ceiling moved"
    );
}

/// A stream under both ceilings is kept whole, with no notice: a reader must be able to trust a
/// short stream, or the notice itself becomes noise.
#[test]
fn a_stream_within_both_ceilings_is_kept_exactly() {
    let mut budget = Budget::new();
    let mut stream = empty_stream("stderr");
    stream.retain(&bytes_of(4096), &mut budget);
    assert_eq!(stream.bytes, bytes_of(4096), "a whole stream was altered");
    assert!(
        !String::from_utf8_lossy(&stream.bytes).contains(NOTICE),
        "a stream within both ceilings was marked truncated, so a reader would distrust it"
    );
}

/// The per-stream ceiling clips a single stream that floods one pipe, and the head survives so
/// #548's refusal attribution still has something to attribute.
#[test]
fn a_flood_on_one_stream_is_clipped_at_the_per_stream_ceiling() {
    let mut budget = Budget::new();
    let mut stream = empty_stream("stderr");
    // Feed far more than the ceiling in realistic read-sized chunks.
    let chunk = bytes_of(64 * 1024);
    for _ in 0..(PER_STREAM / chunk.len()) + 16 {
        stream.retain(&chunk, &mut budget);
    }
    let text = String::from_utf8_lossy(&stream.bytes);
    assert!(
        text.contains(NOTICE),
        "a stream past the per-stream ceiling must end in a truncation notice, or a clipped \
         stream is indistinguishable from a whole one (sts2-harness#559)"
    );
    assert!(
        stream.bytes.len() <= PER_STREAM,
        "retained {} bytes, past the {PER_STREAM}-byte per-stream ceiling",
        stream.bytes.len()
    );
}

/// The shared total ceiling is the axis #559 names and the one with no end-to-end coverage: two
/// streams that are each well *under* the per-stream ceiling together still exceed the total, and
/// the second one to be clipped must say so.
#[test]
fn two_individually_small_streams_are_clipped_at_the_shared_total() {
    let mut budget = Budget::new();
    let mut first = empty_stream("stdout");
    let mut second = empty_stream("stderr");
    let chunk = bytes_of(64 * 1024);
    let rounds = TOTAL / chunk.len() + 16;
    for _ in 0..rounds {
        first.retain(&chunk, &mut budget);
        second.retain(&chunk, &mut budget);
    }
    let held = first.bytes.len() + second.bytes.len();
    assert!(
        held <= TOTAL + 2 * NOTICE.len(),
        "the two streams together retained {held} bytes, past the {TOTAL}-byte shared total \
         (sts2-harness#559)"
    );
    let clipped = String::from_utf8_lossy(&first.bytes).contains(NOTICE)
        || String::from_utf8_lossy(&second.bytes).contains(NOTICE);
    assert!(
        clipped,
        "a capture past the shared total ceiling clipped a stream without saying so, so a \
         reader would take a clipped stream for a whole one (sts2-harness#559)"
    );
}

/// The total budget is spent only by bytes actually retained. A flood that is drained and dropped
/// must not starve the *other* stream's ceiling, or one chatty pipe could silently starve the
/// other pipe's evidence — the same defect this issue is about, one level up.
#[test]
fn dropped_bytes_do_not_spend_the_shared_total() {
    let mut budget = Budget::new();
    let mut loud = empty_stream("stdout");
    let mut quiet = empty_stream("stderr");
    let chunk = bytes_of(64 * 1024);
    // Fill `loud` well past its own per-stream ceiling. Everything past that ceiling is dropped,
    // not retained, and a dropped byte must cost the shared total nothing.
    for _ in 0..(PER_STREAM / chunk.len()) + 16 {
        loud.retain(&chunk, &mut budget);
    }
    // `loud` retained at most `PER_STREAM` plus its notice no matter how much it wrote, so the
    // shared total still has nearly half of itself left.
    let loud_held = loud.bytes.len();
    let available = TOTAL - loud_held;
    assert!(
        available + chunk.len() >= PER_STREAM,
        "a flood that retained only {loud_held} bytes should leave nearly a whole stream's worth \
         of shared budget, but only {available} remained, so the dropped tail was charged"
    );
    // The quiet stream must then be able to spend all of what remains. Retaining the full
    // remainder is only possible if `loud`'s dropped tail cost the shared total nothing: had it
    // been charged, the quiet stream would have been cut short by exactly that dropped amount.
    for _ in 0..(available / chunk.len()) {
        quiet.retain(&chunk, &mut budget);
    }
    assert!(
        quiet.bytes.len() >= available - chunk.len(),
        "the quiet stream retained only {} bytes of the {available} that remained after the loud \
         stream; the loud stream's dropped bytes must not spend the shared total \
         (sts2-harness#559)",
        quiet.bytes.len()
    );
}

/// A stream that is already clipped stops retaining but keeps draining, so it neither grows
/// without bound nor blocks the writer by refusing to read.
#[test]
fn a_clipped_stream_keeps_draining_but_stops_retaining() {
    let mut budget = Budget::new();
    let mut stream = empty_stream("stderr");
    let chunk = bytes_of(64 * 1024);
    for _ in 0..(PER_STREAM / chunk.len()) + 32 {
        stream.retain(&chunk, &mut budget);
    }
    let after_clip = stream.bytes.len();
    // Further reads change nothing: the stream is closed to retention, not grown.
    for _ in 0..32 {
        stream.retain(&chunk, &mut budget);
    }
    assert_eq!(
        stream.bytes.len(),
        after_clip,
        "a clipped stream kept growing past its ceiling"
    );
}
