// SPDX-License-Identifier: MIT

//! Pin that the served gateway's own streams are captured *while it runs*, not after it is
//! killed. Refs sts2-harness#559.
//!
//! # The defect under test
//!
//! `gateway_with_identity` spawns the gateway with `Stdio::piped()` on both streams, but until
//! this fix neither pipe was read until `stop` SIGKILLed the process group and then called
//! `wait_with_output()`. A pipe holds one buffer — 65,536 bytes on Linux — before its writer
//! blocks. So a gateway that wrote more than one buffer was still blocked mid-write at the
//! moment its group was killed, and every byte it had not yet written was discarded: no error,
//! no truncation notice, no diagnostic. The gateway was clipped at exactly one pipe buffer and
//! lost its own tail, which leaves a served failure holding nothing that explains why.
//!
//! Root's independent reproduction, which ran to completion before this test existed, measured
//! that wall directly: a child writing 2 MiB to a pipe with no prior reads yielded **65,536**
//! of **2,097,152** intended bytes after the group was killed — 96.9% silently lost. The
//! number is structural, not a race, which is why the assertion below is a count and not a
//! "the tail is probably there" check.
//!
//! # Why a stub gateway
//!
//! The claim is about this repository's plumbing, not about a real peer. A stub that writes a
//! known number of bytes makes the lost-byte count exact and checkable, where asserting against
//! the real gateway would pass vacuously whenever the peer happened to be quiet. It is spawned
//! through the same `gateway_with_identity` the real compositions use, so it inherits the same
//! cleared environment, the same `process_group(0)`, and the same pipes.
//!
//! The stub deliberately **writes past the pipe buffer and then keeps serving**. That is the
//! whole shape of #559: a chatty gateway that blocks mid-write while `ready` returns, serves
//! nothing for the rest of the scenario, and is then killed. A stub that wrote only a little
//! would fit in one buffer and pass on the old code, which is exactly the case the issue says
//! must fail.
//!
//! None of these tests is `#[ignore]`d, none needs an operator-built peer binary, and none
//! depends on an execution count, so they run in an ordinary `cargo test`.

#![cfg(unix)]

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
// This test binary compiles the shared fixture but exercises only the capture path, so several
// `FixtureMode` variants and fixture helpers are unused here while remaining live in
// `runtime_v4_executable_composition`. The process support module carries its own crate-level
// `allow(dead_code)`, so this suppression is only needed for the fixture and is scoped to this
// binary rather than added to the shared file.
#[allow(dead_code)]
mod fixture;
#[path = "support/runtime_v4_executable_composition_process.rs"]
mod process;
#[path = "support/served_gateway_capture_drain_stub.rs"]
mod stub;

use process::{TempDir, free_address};

use process::{MAX_CAPTURE_BYTES, MAX_TOTAL_CAPTURE_BYTES, TRUNCATION_NOTICE};
use stub::{both_pipes_gateway, chatty_gateway, quiet_gateway};

/// How many bytes the chatty stub writes to its own stderr before it serves.
///
/// Comfortably more than one pipe buffer, so the assertion is about recovering far more than
/// 64 KiB rather than about a marginal difference at the boundary.
pub(crate) const CHATTY_BYTES: usize = 2 * 1024 * 1024;

/// The Linux default pipe capacity. Named so the assertion can *report* the wall it is beating
/// instead of only reporting the failure.
const PIPE_BUFFER_BYTES: usize = 64 * 1024;

/// The marker written once, at the head, before the flood.
///
/// It stands in for the refusal-and-context that #548's attribution depends on, so this test
/// also proves the fix did not buy its tail by dropping the head.
pub(crate) const HEAD_MARKER: &str = "sts2-harness-559-chatty-gateway-head-marker";

/// A gateway that wrote far more than one pipe buffer must have far more than one pipe buffer
/// recovered.
///
/// Acceptance criterion 1. On the pre-#559 shape this recovers at most one pipe buffer, because
/// the only read of the pipe happens after the group is killed, so the left-hand value is
/// `[65,536, 65,536]` and this fails.
#[test]
fn a_chatty_gateway_has_more_than_one_pipe_buffer_recovered()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = chatty_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);

    assert!(
        captured.len() > PIPE_BUFFER_BYTES,
        "the served capture recovered only {} bytes of the {CHATTY_BYTES} the gateway wrote, \
         which is at most one pipe buffer ({PIPE_BUFFER_BYTES}). Nothing drains the pipe while \
         the gateway runs, so a chatty gateway is clipped at one buffer and its tail is lost \
         silently (sts2-harness#559).",
        captured.len()
    );
    assert!(
        captured.len() >= CHATTY_BYTES,
        "the served capture recovered {} of the {CHATTY_BYTES} bytes the gateway wrote, so the \
         tail was still lost (sts2-harness#559)",
        captured.len()
    );
    Ok(())
}

/// The head of the stream must survive, and the tail must be identifiable as a tail.
///
/// This is the property that stops the fix being bought by truncation. #548 depends on the
/// gateway's own refusal reaching the served failure text, and #559 is about the tail that
/// arrives *after* it. Both halves are asserted here: the marker written first is still
/// present, and the bytes that followed it are all there too.
#[test]
fn a_chatty_gateway_keeps_its_head_and_its_tail() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = chatty_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);

    assert!(
        captured.contains(HEAD_MARKER),
        "the served capture lost the head of the gateway's own stream, so #548's attribution \
         regresses into the #559 fix (sts2-harness#559)"
    );
    assert!(
        captured.contains(CHATTY_TAIL_MARKER),
        "the served capture lost the tail of the gateway's own stream: the marker written \
         after {CHATTY_BYTES} bytes of flood is absent, so the stream is still clipped \
         (sts2-harness#559)"
    );
    Ok(())
}

/// A quiet gateway's capture is unchanged by the drain.
///
/// The fix adds a background drain to every served gateway, so the ordinary case has to be
/// pinned too: a gateway that says almost nothing must still report exactly what it said.
/// Without this, "recovered more than a pipe buffer" could be satisfied by a capture that
/// returns unrelated bytes.
#[test]
fn a_quiet_gateway_still_reports_exactly_what_it_wrote() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = quiet_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;
    let captured = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        captured,
        format!("{QUIET_MARKER}\n"),
        "the served capture altered a quiet gateway's own stream"
    );
    Ok(())
}

/// The marker written *after* the flood, so the tail's arrival is a positive fact rather than
/// an inference from a byte count.
pub(crate) const CHATTY_TAIL_MARKER: &str = "sts2-harness-559-chatty-gateway-tail-marker";

/// A quiet gateway's only line.
pub(crate) const QUIET_MARKER: &str = "sts2-harness-559-quiet-gateway-marker";

/// #567's end-to-end case: a served gateway that writes past the **shared** total across **both**
/// pipes.
///
/// The unit-level tests in `gateway_capture/stream_tests.rs` drive `Stream::retain` directly,
/// which is where the accounting lives, but #567's criterion 1 asks for something narrower and
/// different: a *served gateway* that writes more than 8 MiB **across both pipes**. This stub is
/// that gateway. It is spawned through the same `gateway_with_identity` the real compositions
/// use, so it inherits the same pipes, the same `process_group(0)`, and the same shared budget.
///
/// Each pipe is flooded to *past the per-stream ceiling on its own* and past the total together.
/// That is deliberate. A stub that flooded one pipe to 5 MiB and left the other empty would never
/// touch the shared budget, so it would pass on a build whose total ceiling did not exist.
pub(crate) const BOTH_PIPES_BYTES: usize = 6 * 1024 * 1024;

/// #567 acceptance criterion 1, end to end: the pair the harness hands back never exceeds the
/// shared total, each cut stream announces itself, and the notice is paid for **inside** the
/// total rather than added on top of it.
///
/// The excess is checked as `held <= MAX_TOTAL_CAPTURE_BYTES` rather than `held == total - notice`
/// because the exact number depends on how the two drains interleave — whichever pipe happens to
/// be drained first spends the budget first. The load-bearing half of this assertion is the
/// `held` side: a build that charged the notice *after* the total was computed hands back
/// `total + notice`, and fails here. The unit test in `stream_tests.rs` pins the exact figure for
/// a single drain order; this one pins the bound that must hold for every one of them.
#[test]
fn a_gateway_flooding_both_pipes_is_bounded_by_the_shared_total()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = both_pipes_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    // Wait for the flood itself, not just for the port. The stub binds before it floods (see
    // `BIND_LISTENER`), so `ready` now returns while the flood is still being written; stopping
    // here would measure how much the drain took before the kill rather than the shared ceiling.
    // Both pipes carry `BOTH_PIPES_BYTES` apiece, and the count is of bytes the drain *read*,
    // including bytes it later drops at a ceiling, so this threshold is reached only once every
    // flooded byte has actually crossed the pipe. It is deliberately the sum of both pipes and not
    // one pipe's worth, so a stub that flooded only one side could not satisfy it.
    gateway_process.await_bytes(2 * BOTH_PIPES_BYTES, std::time::Duration::from_secs(30))?;
    let output = process::stop(gateway_process)?;

    let stdout = &output.stdout;
    let stderr = &output.stderr;
    let held = stdout.len() + stderr.len();

    assert!(
        held <= MAX_TOTAL_CAPTURE_BYTES,
        "the served capture returned {held} bytes across both pipes, over the shared ceiling of \
         {MAX_TOTAL_CAPTURE_BYTES} (sts2-harness#567)"
    );
    assert!(
        held > MAX_CAPTURE_BYTES,
        "the served capture returned only {held} bytes across both pipes, so the shared total \
         of {MAX_TOTAL_CAPTURE_BYTES} was never reached and this test proves nothing \
         (sts2-harness#567)"
    );

    // Both pipes were flooded past the per-stream ceiling, so both carry a notice. A build that
    // announced neither is handing a reader a capture that looks whole when the pair was cut.
    for (label, stream) in [("stdout", stdout), ("stderr", stderr)] {
        assert!(
            stream
                .windows(TRUNCATION_NOTICE.len())
                .any(|window| window == TRUNCATION_NOTICE),
            "the {label} pipe was flooded to {BOTH_PIPES_BYTES} bytes, over the per-stream ceiling \
             of {MAX_CAPTURE_BYTES}, yet its capture carries no truncation notice. A reader is \
             handed a stream that looks whole when it was cut (sts2-harness#567)"
        );
    }
    Ok(())
}

/// The control for the test above: a gateway that stays inside the shared total carries **no**
/// notice on either pipe.
///
/// Without this, "every served gateway carries a notice" would satisfy the acceptance criterion —
/// a capture that announces truncation unconditionally is trivially bounded and useless.
#[test]
fn a_gateway_inside_the_shared_total_carries_no_truncation_notice()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let stub = quiet_gateway(&temporary.path)?;
    let address = free_address()?;
    let mod_address = free_address()?;

    let mut gateway_process = process::gateway(&stub, address, mod_address)?;
    process::ready(&mut gateway_process, address)?;
    let output = process::stop(gateway_process)?;

    for (label, stream) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        assert!(
            !stream
                .windows(TRUNCATION_NOTICE.len())
                .any(|window| window == TRUNCATION_NOTICE),
            "the {label} pipe carried a truncation notice although the gateway stayed inside \
             every ceiling, so a reader cannot tell a cut capture from a whole one \
             (sts2-harness#567)"
        );
    }
    Ok(())
}
