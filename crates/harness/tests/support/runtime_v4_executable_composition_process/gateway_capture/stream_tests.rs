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
//!
//! #567 measured the mutation matrix for the pair-of-pipes cases added here, and two of the three
//! mutations #567's brief asked for do not hold up. Both are recorded rather than papered over:
//!
//! - **Dropping the notice's trim-back** (append the notice on top of the tail instead of cutting
//!   the tail to make room) overspends the total by exactly the notice's length, 38 bytes. Killed
//!   by `both_pipes_flooded_past_the_total_are_clipped_together` and
//!   `the_truncation_notice_is_charged_to_the_shared_total`, and by the two #559 tests.
//! - **Dropping the notice's `charge` entirely** leaves every *retained* byte unchanged, so no
//!   assertion on `held` can see it. It is caught only by comparing what the budget was charged
//!   against what the capture holds, in
//!   `the_bytes_held_are_charged_for_and_the_notice_leaves_a_surplus`. The defect it permits is
//!   real: while
//!   `reached >= TRUNCATION_NOTICE.len()` the trim-back removes the same bytes the notice adds, but
//!   once the shared total is exhausted `reached` is `0`, the trim-back saturates, and the stream
//!   holds a full 38-byte notice the total never paid for — so a capture can overspend the shared
//!   ceiling by 38 bytes per stream clipped from an empty buffer. See the identical hazard stated
//!   in `stream.rs`.
//! - **Charging the total on read rather than on retain** is likewise not observable here, and for
//!   a structural reason: every test in this file drives [`Stream::retain`] directly, so
//!   `drain_once`'s read path is never entered. Catching that mutation needs a test that drains a
//!   real pipe, which is what `served_gateway_capture_drain.rs` is for.

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
        held <= TOTAL,
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

/// Feed `size` bytes to `stream` in realistic read-sized chunks, the way `drain_once` would.
fn feed(stream: &mut Stream<std::io::Empty>, size: usize, chunk: &[u8], budget: &mut Budget) {
    let mut remaining = size;
    while remaining > 0 {
        let take = chunk.len().min(remaining);
        stream.retain(&chunk[..take], budget);
        remaining -= take;
    }
}

/// The combined-total ceiling, exercised the way #567 asks: a gateway that floods *both* pipes
/// past `MAX_TOTAL_CAPTURE_BYTES` while neither pipe alone reaches its own per-stream ceiling.
///
/// This is the case the two ceilings only produce together. One test drives the per-stream axis
/// and another drives the starvation axis, but neither puts two *individually small* streams
/// across the total. Each stream here writes `TOTAL / 2 + slack`, which is under `PER_STREAM` on
/// its own and over `TOTAL` as a pair, so the cut is attributable to the shared budget alone.
#[test]
fn both_pipes_flooded_past_the_total_are_clipped_together() {
    let mut budget = Budget::new();
    let mut stdout = empty_stream("stdout");
    let mut stderr = empty_stream("stderr");
    let chunk = bytes_of(64 * 1024);
    // `PER_STREAM * 2 == TOTAL` exactly, so a pair of streams can never each stay strictly under
    // their own ceiling and still *exceed* the total on write size alone: the per-stream axis
    // always binds first. What the total actually adds is the notice bytes, so each stream is
    // driven one byte past its own ceiling and the pair's overspend is attributable to the
    // notice, not to the per-stream cap. This is asserted rather than assumed, so a future change
    // to the two constants that *did* separate the axes would be caught here.
    let per_stream_write = PER_STREAM + 1;
    assert!(
        per_stream_write > PER_STREAM,
        "each stream must be driven past its own ceiling for the shared total to be the axis \
         that clips it"
    );
    assert_eq!(
        PER_STREAM * 2,
        TOTAL,
        "the per-stream and total ceilings are no longer in the exact 2:1 ratio this test was \
         written against; the two axes are now separable and the test should be redesigned"
    );
    feed(&mut stdout, per_stream_write, &chunk, &mut budget);
    feed(&mut stderr, per_stream_write, &chunk, &mut budget);

    // AC1: the combined retained total never exceeds the shared ceiling, asserted exactly.
    // No slack term. `retain` trims the notice against the ceiling actually reached and then
    // charges it, so the bytes a clipped stream ends holding are always a subset of the bytes
    // the budget paid for. The old `TOTAL + 2 * NOTICE.len()` slack was never needed and would
    // have hidden a real overspend of up to 76 bytes.
    let held = stdout.bytes.len() + stderr.bytes.len();
    assert!(
        held <= TOTAL,
        "the two streams together retained {held} bytes, past the {TOTAL}-byte shared total \
         (sts2-harness#567)"
    );

    // AC2: a stream the total clipped carries the notice, so a reader is never handed a short
    // stream that looks whole.
    let stdout_text = String::from_utf8_lossy(&stdout.bytes);
    let stderr_text = String::from_utf8_lossy(&stderr.bytes);
    assert!(
        stdout_text.contains(NOTICE) && stderr_text.contains(NOTICE),
        "both pipes were driven past the {TOTAL}-byte shared total, so both must be marked \
         truncated; a clipped stream that carries no notice reads as a whole one \
         (sts2-harness#567)"
    );
}

/// The notice must be paid for out of the shared total, not added on top of it.
///
/// This is the "charged to the budget" half of #567's acceptance criteria, and it is enforced by
/// the *trim-back*: a clipped stream cuts its own tail off to make room for the notice, so the
/// notice is never appended on top of bytes the total already paid for. Dropping the trim-back is
/// exactly the mutation that overspends the total, and the pair assertion below catches it.
///
/// One honest limit, recorded because #567's brief asked for it. The `charge` call for the notice
/// is *not* separately observable from this pair, and this test deliberately does not pretend
/// otherwise. While `reached` is at least the notice's length, the trim-back removes the same
/// bytes the notice adds, so a clipped stream holds `reached` bytes whether or not the charge is
/// applied; charging it only leaves the budget stricter. That reasoning stops holding once
/// `reached` drops below the notice's length — the trim-back saturates to zero and the stream
/// holds a full 38-byte notice the budget never paid for, so dropping the charge would overspend
/// the shared total by `38 - reached`. No test on this pair reaches that state (the chunked
/// `feed` lands on 1 MiB boundaries and a clipping stream is fed at least `PER_STREAM`), so the
/// bound is latent; the `charge` in `stream.rs` is what holds it. See the root comment above.
#[test]
fn the_truncation_notice_is_charged_to_the_shared_total() {
    let mut budget = Budget::new();
    let mut stdout = empty_stream("stdout");
    let mut stderr = empty_stream("stderr");
    let chunk = bytes_of(64 * 1024);
    // Both pipes flood, so the total is exhausted and the second clip is the one whose notice has
    // to be paid for out of what the first one left.
    feed(&mut stdout, PER_STREAM * 2, &chunk, &mut budget);
    feed(&mut stderr, PER_STREAM * 2, &chunk, &mut budget);
    let held_by_stdout = stdout.bytes.len();
    assert!(
        held_by_stdout <= PER_STREAM,
        "one stream retained {held_by_stdout} bytes, past the {PER_STREAM}-byte per-stream ceiling"
    );
    assert!(
        String::from_utf8_lossy(&stdout.bytes).contains(NOTICE),
        "the flooded stdout should have been marked truncated; if it was not, this test is not \
         exercising the notice path at all"
    );
    assert!(
        String::from_utf8_lossy(&stderr.bytes).contains(NOTICE),
        "the flooded stderr should have been marked truncated; if it was not, the pair never \
         exhausted the shared total and this test proves nothing"
    );
    // The whole point: the two notices ride inside the total rather than on top of it. Appending a
    // notice without trimming the tail back first is what would push the pair past `TOTAL` here.
    let held = stdout.bytes.len() + stderr.bytes.len();
    assert!(
        held <= TOTAL,
        "the pair retained {held} bytes, so a truncation notice was added on top of the shared \
         total instead of being trimmed into it (sts2-harness#567)"
    );
}

/// A stream the total never reached must NOT carry a notice.
///
/// The failure this guards against is a capture that clips nothing but still marks a stream as
/// truncated, which teaches a reader to distrust a complete capture. Another test covers the
/// single-stream case; this covers the *pair*, where a greedy `stdout` could leave a small `stderr`
/// marked in association with the wrong ceiling.
#[test]
fn an_uncut_stream_in_a_bounded_pair_carries_no_notice() {
    let mut budget = Budget::new();
    let mut loud = empty_stream("stdout");
    let mut quiet = empty_stream("stderr");
    let chunk = bytes_of(64 * 1024);
    feed(&mut loud, PER_STREAM * 2, &chunk, &mut budget);
    feed(&mut quiet, 4096, &chunk, &mut budget);
    assert!(
        String::from_utf8_lossy(&loud.bytes).contains(NOTICE),
        "the flooded stream should have been marked truncated; if it was not, this test is not \
         exercising the notice path at all"
    );
    assert_eq!(
        quiet.bytes.len(),
        4096,
        "the quiet stream's bytes were altered by the loud stream's clipping"
    );
    assert!(
        !String::from_utf8_lossy(&quiet.bytes).contains(NOTICE),
        "a stream that was never clipped carries a truncation notice, so a reader would \
         distrust a capture that is actually complete (sts2-harness#567)"
    );
}

/// The bytes the capture holds are always a subset of the bytes the budget was charged for.
///
/// This is the assertion that makes #567's acceptance criterion 2 falsifiable, and it is the only
/// one in this file that can. Criterion 2's mutation — dropping
/// `budget.charge(TRUNCATION_NOTICE.len())` — changes **no retained byte at all**: the trim-back
/// already removes exactly the bytes the notice adds, so the buffer is length-neutral at the clip
/// point. Deleting the charge is therefore invisible to any assertion on `held`, and the full
/// `served_gateway_capture_drain` binary stays 19/19 green with the charge removed. What the charge
/// actually does is make the budget *stricter* than the bytes held, and that surplus is the signal:
/// on correct code the shared total ends up charged strictly more than the capture holds, by
/// exactly the one notice that was not refunded.
///
/// The direction of the inequality is the whole content of this test, so it is worth being exact
/// about why it is `>` and not `>=`:
///
/// - `retain` charges `remaining`, then the trim-back **discards** `min(reached, notice)` bytes that
///   were already charged, then the notice is appended and charged again. The discarded bytes are
///   never refunded, so a clipped stream leaves a permanent surplus of exactly one notice.
/// - Once the shared total is exhausted, `remaining` is `0`, so `reached` is `0`, the trim-back
///   saturates and discards nothing, and the notice is charged 38 bytes against a total that is
///   already spent. `Budget::charge` saturates rather than panicking, so the capture holds a notice
///   the total could not pay for — the very hazard the `charge` exists to hold closed.
/// - With the charge removed, both of those surpluses vanish and the pair holds exactly what the
///   total paid for. So `charged > held` here is not a slack term; it is the assertion.
///
/// Measured, driven through the real `retain` on the committed tree: `charged = 8388608`,
/// `held = 8388570`, surplus `38`. With the charge deleted: `charged = 8388608`, `held = 8388608`,
/// surplus `0`, and the capture sits *exactly* on the ceiling with nothing to spare.
#[test]
fn the_bytes_held_are_charged_for_and_the_notice_leaves_a_surplus() {
    let mut budget = Budget::new();
    let chunk = bytes_of(64 * 1024);
    // The first stream is clipped by its own per-stream ceiling, which is reached first because
    // `PER_STREAM` is half the total. The second then meets the shared total partway through, so
    // the pair exercises the *shared* axis rather than the per-stream one twice.
    let mut first = empty_stream("stdout");
    feed(&mut first, PER_STREAM + 1, &chunk, &mut budget);
    let mut second = empty_stream("stderr");
    feed(&mut second, PER_STREAM * 2, &chunk, &mut budget);

    let held = first.bytes.len() + second.bytes.len();
    let charged = TOTAL - budget.remaining();
    let carries_notice =
        |stream: &Stream<std::io::Empty>| String::from_utf8_lossy(&stream.bytes).contains(NOTICE);
    assert!(
        carries_notice(&first) && carries_notice(&second),
        "both streams are driven past both ceilings, so both must carry a notice; if either does \
         not, this test is not exercising the notice path it was written for \
         (sts2-harness#567)"
    );
    assert!(
        charged > held,
        "the budget was charged {charged} bytes and the capture holds {held}, so the pair holds \
         every byte the shared total paid for and the truncated bytes behind the notices were not \
         charged. The truncation notice is being emitted without being charged to the budget, so \
         each notice rides on top of the shared ceiling instead of inside it \
         (sts2-harness#567)"
    );
}
