// SPDX-License-Identifier: MIT

//! Running `gh` as a child process: spawning it safely, draining its pipes, and
//! bounding how long it may take.
//!
//! Split out of `main.rs` so the policy in `decision.rs` and the process lifecycle
//! here move independently. Everything in this module is about the *transport*:
//! what the `gh` seam must do for the call to be correct, bounded, and free of the
//! kernel races the tests exercise. Nothing here decides anything about a review.

use crate::decision::ReviewGateError;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// `ETXTBSY`, the errno `execve` returns when the image is still open for writing anywhere.
pub(crate) const TEXT_FILE_BUSY: i32 = 26;

/// How long to keep re-trying before giving up and surfacing the original error.
const DEADLINE: Duration = Duration::from_secs(10);

/// How long to wait between attempts. The holder of the descriptor closes it within
/// microseconds, so a long backoff would only add latency; short is bounded by the deadline.
const BACKOFF: Duration = Duration::from_millis(2);

/// How often the child is polled while waiting for it.
///
/// Short enough that a timeout is not reported materially later than it happened,
/// long enough that the wait is not a busy loop.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Run `gh` for `endpoint`, bounded by `timeout`, returning its status and captured output.
///
/// The stdio wiring is the same `Command::output()` applies, which a bare `spawn` does
/// not: stdout and stderr are captured for the caller's own error reporting, and stdin is
/// closed rather than inherited so `gh` can never block waiting on the gate's terminal.
pub(crate) fn run(
    gh_path: &str,
    endpoint: &str,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), ReviewGateError> {
    let mut command = Command::new(gh_path);
    command
        .arg("api")
        .arg(endpoint)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Spawned through the `ETXTBSY` retry rather than a bare `spawn()`: see
    // [`spawn_retrying_text_busy`], and #707 for why a bare spawn is racy here.
    let mut child = spawn_retrying_text_busy(&mut command)
        .map_err(|error| ReviewGateError(format!("unable to run {gh_path}: {error}")))?;

    // Both pipes are drained on their own threads. Reading them inline after
    // the child exits would deadlock the other way round: a child that fills a
    // pipe buffer blocks in `write` while we block in `read`, and neither side
    // makes progress. Draining concurrently lets the child always finish
    // writing, however large the body is.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || read_pipe(stdout));
    let stderr_reader = thread::spawn(move || read_pipe(stderr));

    let status = wait_with_timeout(&mut child, endpoint, timeout)?;
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok((status, stdout, stderr))
}

/// Wait for `child`, or report the deadline that elapsed first.
fn wait_with_timeout(
    child: &mut Child,
    endpoint: &str,
    timeout: Duration,
) -> Result<ExitStatus, ReviewGateError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            // Still running. Fall through to the deadline check.
            Ok(None) => {}
            Err(error) => {
                reap(child);
                return Err(ReviewGateError(format!(
                    "unable to wait for the `gh api` call for {endpoint}: {error}"
                )));
            }
        }
        if Instant::now() >= deadline {
            reap(child);
            return Err(ReviewGateError(format!(
                "gh api timed out after {}s for {endpoint}",
                timeout.as_secs()
            )));
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    }
}

/// Spawn `command`, retrying only while it is refused with `ETXTBSY`.
///
/// # Why this is needed
///
/// `execve` refuses to run a file that any live descriptor holds open for writing, and returns
/// `ETXTBSY` (`errno 26`) when it does. The stubs this crate's own tests execute are produced by
/// [`fs::write`](std::fs::write) followed by `chmod 0o755` and then exec, and the test binary runs
/// its tests in parallel threads. So while thread A is between its own `write` and its own `exec`,
/// thread B can `fork`, inherit that still-open write descriptor, and `exec` -- and whichever
/// `exec` lands first loses.
///
/// This is a genuine kernel race, not a defect in the stubs. Each stub gets its own directory and
/// filename, so the two threads never touch the same path; the coupling is the inherited
/// descriptor, not the name. It is rare enough to be unreproducible on demand: #707 records one
/// failure in 40 full-suite runs, and 200 consecutive clean full-suite runs on this host did not
/// reproduce it at all. That rarity is why it survived a fix that removed a genuine path collision
/// and then declared victory on 12 clean runs -- a rate that cannot distinguish "fixed" from
/// "almost never hit", which is why the regression test manufactures the condition instead of
/// looping the suite and hoping. Because `review-of-record` is a required check on every pull
/// request, an occurrence blocks unrelated work at random, after the author has already done the
/// right thing, which trains re-running instead of reading. See #707.
///
/// This is the same remedy the harness test support already uses, and for the same measured
/// reason: `crates/harness/tests/support/runtime_v4_executable_composition_process/spawn.rs`
/// measured **5 failures in 240 spawns** without the retry and **0 in 240** with it.
///
/// Retrying is sound because the descriptor that caused the refusal is always closed by its
/// owner -- `fs::write` returns only after the `File` drops -- so the condition is transient by
/// construction and the deadline cannot be outlived by a *persistent* one. A script held open for
/// writing by some other process would instead burn the full [`DEADLINE`] and then report the
/// original `ETXTBSY` rather than hanging forever, and any other errno propagates on the first
/// attempt with no delay at all.
pub(crate) fn spawn_retrying_text_busy(command: &mut Command) -> Result<Child, std::io::Error> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.raw_os_error() == Some(TEXT_FILE_BUSY) && Instant::now() < deadline =>
            {
                thread::sleep(BACKOFF);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Drain one pipe to a `Vec<u8>` on a dedicated thread.
///
/// A read error yields whatever was collected before it, which is the same
/// fail-closed direction as every other error here: a truncated body cannot parse
/// as the JSON the caller requires, so a partial read becomes a reported failure
/// rather than a silently short one.
fn read_pipe<R: Read + Send + 'static>(pipe: Option<R>) -> Vec<u8> {
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut collected = Vec::new();
    let _ = pipe.read_to_end(&mut collected);
    collected
}

/// Kill a child and reap it, so a timeout does not leave a process behind.
fn reap(child: &mut Child) {
    // Both calls are best-effort by design. A child that already exited between the
    // deadline check and the kill reports failure here; the `wait` that follows is
    // still correct, because it is the call that reaps.
    let _ = child.kill();
    let _ = child.wait();
}
