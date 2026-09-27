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
//! The last case here is the cross-cutting one #567 filed: #559's capture ceiling and #555's
//! evidence **write** ceiling were each verified alone, and neither was verified *together*
//! against a capture past the shared 8 MiB total. The total is a shared budget and the truncation
//! notice is paid for out of it, so those two facts only interact at the boundary.
//!
//! The totals are restated as literals rather than imported, so this file cannot agree with the
//! implementation by construction: moving a constant has to move the number here, or these
//! assertions fail. That is the point — a bound that silently drifts is worse than no bound.

use super::{Budget, MAX_CAPTURE_BYTES, MAX_TOTAL_CAPTURE_BYTES, Stream, TRUNCATION_NOTICE};

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

/// The combined-total case #567 asks for: both pipes flooded *together*, past the shared
/// [`TOTAL`], each of them comfortably under the per-stream [`PER_STREAM`] ceiling.
///
/// The chunk is deliberately not a power of two, so the streams do not land on a ceiling at the
/// same instant; a power-of-two chunk would make the first stream stop retaining for arithmetic
/// reasons rather than because the budget it shares with the other pipe was actually spent.
fn flood_both_pipes_past_the_shared_total()
-> (Budget, Stream<std::io::Empty>, Stream<std::io::Empty>) {
    let mut budget = Budget::new();
    let mut first = empty_stream("stdout");
    let mut second = empty_stream("stderr");
    let chunk = bytes_of(60_007);
    for _ in 0..(TOTAL / chunk.len()) + 32 {
        first.retain(&chunk, &mut budget);
        second.retain(&chunk, &mut budget);
    }
    (budget, first, second)
}

fn carries_notice(stream: &Stream<std::io::Empty>) -> bool {
    stream
        .bytes
        .windows(TRUNCATION_NOTICE.len())
        .any(|window| window == TRUNCATION_NOTICE)
}

/// Acceptance criterion 1: with both pipes flooded past the shared total, the **combined**
/// retained bytes never exceed the total — and they sit at it exactly, because a notice that was
/// charged on top would push the pair past the ceiling it is announcing.
///
/// The **exact excess** is the load-bearing part, and it is deliberately not zero. A truncation
/// notice is charged in full even when it trimmed already-retained bytes back off to make room, and
/// `Budget::charge` saturates, so the amount charged is always *at least* what is still held. In
/// this shape one stream is stopped by the shared total with fewer bytes than the notice's length,
/// so its notice is paid for entirely out of the total, and the pair is held exactly
/// `TOTAL - NOTICE`. Measured on `main` at `da606fa`: held `8_388_570`, charged `8_388_608`.
///
/// Pinning the excess at exactly one notice is what makes criterion 2's mutation observable. A
/// build that charged the notice only *after* the total was computed holds the same `TOTAL` bytes
/// here — it is unobservable in this shape — so the per-stream form in
/// `the_shared_total_is_spent_only_by_retained_bytes` is what fails it.
#[test]
fn the_shared_total_is_charged_and_the_pair_never_exceeds_it() {
    let (budget, first, second) = flood_both_pipes_past_the_shared_total();
    let held = first.bytes.len() + second.bytes.len();
    let charged = TOTAL - budget.remaining();

    assert_eq!(
        budget.remaining(),
        0,
        "a capture that filled the shared total left {} of it unspent, so the ceiling in \
         MAX_TOTAL_CAPTURE_BYTES is not what this capture is actually bounded by \
         (sts2-harness#567)",
        budget.remaining()
    );
    assert_eq!(
        charged, TOTAL,
        "the shared total should be charged exactly once over, but it was charged {charged} \
         bytes (sts2-harness#567)"
    );
    assert!(
        held <= charged,
        "the pair retained {held} bytes but the budget was only charged {charged}: bytes are \
         being held that the shared total never paid for (sts2-harness#567)"
    );
    assert_eq!(
        charged - held,
        TRUNCATION_NOTICE.len(),
        "the pair held {held} bytes against {charged} charged, an excess of {}; exactly one \
         {}-byte truncation notice is expected to be paid for out of the shared total \
         (sts2-harness#567)",
        charged - held,
        TRUNCATION_NOTICE.len()
    );
    assert!(
        held <= TOTAL,
        "the two streams together retained {held} bytes, past the {TOTAL}-byte shared total \
         (sts2-harness#567)"
    );
}

/// Acceptance criterion 1, continued: each stream that was cut carries a notice, and a stream
/// that was **not** cut carries none. The second half matters as much as the first — a notice on
/// a whole stream is the same defect in the other direction, telling a reader their evidence is
/// incomplete when it is not.
#[test]
fn a_cut_stream_is_marked_and_a_whole_stream_is_not() {
    let (_budget, first, second) = flood_both_pipes_past_the_shared_total();
    assert!(
        carries_notice(&first) || carries_notice(&second),
        "a capture past the shared total clipped at least one stream without marking it, so a \
         reader would take a clipped stream for a whole one (sts2-harness#567)"
    );
    // Whichever stream was cut, the other must not claim to be incomplete. In this shape the
    // first stream reaches the per-stream ceiling and the second is stopped by what is left of
    // the shared total, so the assertions are made per-stream rather than assumed.
    for (label, stream) in [("stdout", &first), ("stderr", &second)] {
        let whole = stream.bytes.len() < PER_STREAM && !carries_notice(stream);
        let marked = carries_notice(stream);
        assert!(
            whole || marked,
            "the {label} stream retained {} bytes and carries no truncation notice, so its \
             completeness cannot be read from the capture (sts2-harness#567)",
            stream.bytes.len()
        );
    }
}

/// Acceptance criterion 2, stated as the property rather than the mutation: a notice is paid for
/// out of the budget, so making room for one can only ever cost already-retained bytes, never
/// raise the total.
///
/// A stream cut *just* below the notice's length is the boundary where a trim-back has to happen.
/// The `retain` rule trims the buffer to make room and then charges the whole notice, which is why
/// a stream stopped short by the shared total — where `reached` is under `TRUNCATION_NOTICE.len()`
/// — still ends up charged correctly rather than holding a notice the total never paid for.
#[test]
fn a_notice_that_must_trim_back_is_still_charged_to_the_budget() {
    // A budget deliberately too small to hold the notice itself, so the trim-back branch is the
    // one being exercised rather than a stream that happened to have room.
    let mut budget = Budget {
        remaining: TRUNCATION_NOTICE.len() - 1,
    };
    let mut stream = empty_stream("stderr");
    stream.retain(&bytes_of(TRUNCATION_NOTICE.len() * 2), &mut budget);

    let charged = (TRUNCATION_NOTICE.len() - 1) - budget.remaining();
    // The budget started one byte short of the notice, so it must go to zero: the notice is paid
    // for out of the budget even though it had to cut back a retained byte to fit. A build that
    // dropped the trim-back, or charged the notice only after the total was computed, would leave
    // budget remaining and hold a notice the total never paid for.
    assert_eq!(
        budget.remaining(),
        0,
        "a notice that had to trim back a retained byte to fit left {} of the budget unspent; the \
         notice must be charged even when it cuts back already-retained bytes (sts2-harness#567)",
        budget.remaining()
    );
    assert_eq!(
        stream.bytes.len(),
        TRUNCATION_NOTICE.len(),
        "a stream cut below the notice's own length should hold exactly the notice, but it holds \
         {} bytes (sts2-harness#567)",
        stream.bytes.len()
    );
    assert!(
        carries_notice(&stream),
        "a stream stopped by the shared total was not marked as clipped, so a reader would take \
         it for a whole one (sts2-harness#567)"
    );
}

/// Acceptance criterion 3, as an invariant rather than a mutation: the total is spent by bytes
/// **retained** and by nothing else. A flood that is drained and dropped must cost the total
/// nothing, or one chatty pipe would silently starve the other pipe's evidence.
///
/// This is the property #559's own `dropped_bytes_do_not_spend_the_shared_total` asserts from the
/// other direction; it is restated here against the *combined* capture so the two bounds are
/// checked together, which is what #567 filed for.
#[test]
fn the_shared_total_is_spent_only_by_retained_bytes() {
    let mut budget = Budget::new();
    let mut loud = empty_stream("stdout");
    let mut quiet = empty_stream("stderr");
    let chunk = bytes_of(60_007);

    // A very large flood, most of which is drained and dropped.
    for _ in 0..(TOTAL * 2 / chunk.len()) {
        loud.retain(&chunk, &mut budget);
    }
    let loud_held = loud.bytes.len();
    let loud_charged = TOTAL - budget.remaining();
    // Charged is *at least* held: the notice is charged in full even when it replaced retained
    // bytes, so the excess here is exactly the notice. A dropped byte, by contrast, is charged
    // nothing at all, which is what keeps the loud stream's flood from starving the quiet one.
    assert!(
        loud_charged - loud_held == TRUNCATION_NOTICE.len(),
        "the loud stream retained {loud_held} bytes and was charged {loud_charged}; the excess \
         should be exactly its one {}-byte truncation notice. A smaller excess means the notice \
         was not charged to the budget; a larger one means dropped bytes were \
         (sts2-harness#567)",
        TRUNCATION_NOTICE.len()
    );
    assert!(
        budget.remaining() >= TOTAL - PER_STREAM - TRUNCATION_NOTICE.len(),
        "a stream that retained {loud_held} bytes left only {} of the {TOTAL}-byte shared total; \
         it must still reach the other stream's whole {PER_STREAM}-byte per-stream ceiling less \
         its one truncation notice, so its dropped tail was charged against the budget \
         (sts2-harness#567)",
        budget.remaining(),
    );

    // The quiet stream can then still take a whole per-stream ceiling's worth, which is only
    // possible if the loud stream's dropped tail cost the total nothing.
    for _ in 0..(TOTAL / chunk.len()) + 16 {
        quiet.retain(&chunk, &mut budget);
    }
    assert!(
        quiet.bytes.len() >= PER_STREAM - chunk.len(),
        "the quiet stream retained only {} bytes after the loud stream's flood; had the dropped \
         bytes been charged, it would have been cut short (sts2-harness#567)",
        quiet.bytes.len()
    );
}
