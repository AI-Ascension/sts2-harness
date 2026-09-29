// SPDX-License-Identifier: MIT

//! The malformed-body case of the System One bridge process suite, split out to keep that binary
//! inside its preferred size budget. It is here rather than inline because it is the only case
//! that needs a transport which SUCCEEDS and answers wrongly, and that pairing is what it drives.

use super::scratch::Scratch;
use super::{refused_without_a_decision, run_with};

/// A transport that SUCCEEDS with a malformed body is reported as the parse error, never as a
/// broken pipe.
///
/// This is the instance #753 filed. #752 reordering the exit-status check fixed the *refusal*
/// case, but on a successful exit the writer's `EPIPE` still pre-empted the body-parse error, so
/// whether the bridge named the real cause depended on whether the child exited before or after
/// `write_all` finished -- a scheduling race, measured at 21/40 wrong on `main` at `82259540`.
///
/// The assertion is deliberately an INVARIANT and not the racy outcome: `Broken pipe` must never
/// appear, and the parse error must be what gets named. Pinning either outcome alone would flake
/// in the direction it forbids, which is the mistake that produced the original vacuous regression
/// test on this defect class. The case runs the exchange REPEATEDLY and requires every run to
/// agree, so a reintroduced race surfaces as disagreement rather than one lucky sample.
#[test]
fn a_successful_transport_with_a_malformed_body_is_reported_as_the_parse_error()
-> Result<(), String> {
    const RUNS: usize = 12;
    let mut causes: Vec<String> = Vec::new();
    for run in 0..RUNS {
        let cause = cause_for_a_successful_malformed_body(run)?;
        if cause.contains("Broken pipe") {
            return Err(format!(
                "a transport that SUCCEEDED with a malformed body was reported as a broken pipe \
                 instead of the parse error, on run {run} of {RUNS}: {cause:?}"
            ));
        }
        if !cause.contains("key must be a string") {
            return Err(format!(
                "a malformed body was not reported as the parse error, on run {run} of {RUNS}: \
                 {cause:?}"
            ));
        }
        causes.push(cause);
    }
    // Every run must name the same cause. A race would show up here as disagreement, which is the
    // property that actually distinguishes "fixed" from "one lucky sample".
    if causes.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(format!(
            "the reported cause was not stable across {RUNS} identical runs, so the race is still \
             present: {causes:?}"
        ));
    }
    Ok(())
}

/// One run of a transport that succeeds with a malformed body, returning the cause it named.
///
/// The transport never reads its stdin and exits 0, so the status is a success and only the body
/// is wrong. The request is padded past the pipe buffer, so the writer's `write_all` has something
/// to lose and the race this drives is real rather than absorbed by the buffer.
fn cause_for_a_successful_malformed_body(run: usize) -> Result<String, String> {
    let scratch = Scratch::new(&format!("malformed-body-{run}"))?;
    let transport =
        scratch.transport_refusing_without_reading("malformed.sh", "printf '{not json'\nexit 0")?;
    let output = run_with(
        &transport,
        &[],
        &[],
        &Scratch::request_past_the_pipe_buffer(),
    )?;
    if !scratch.invoked() {
        return Err("malformed.sh never ran, so this case proved nothing".to_owned());
    }
    refused_without_a_decision(&output)?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("cause: "))
        .map(str::to_owned)
        .ok_or_else(|| format!("no cause was named for a malformed body: {stderr:?}"))
}
