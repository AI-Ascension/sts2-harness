// SPDX-License-Identifier: MIT

use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

/// `ETXTBSY`, the errno `execve` returns when the image is still open for writing anywhere.
const TEXT_FILE_BUSY: i32 = 26;

/// How long to keep re-trying before giving up and surfacing the original error.
const DEADLINE: Duration = Duration::from_secs(10);

/// How long to wait between attempts. The holder of the descriptor closes it within
/// microseconds, so a long backoff would only add latency; short is bounded by the deadline.
const BACKOFF: Duration = Duration::from_millis(2);

/// `command.spawn()` with a bounded retry on `ETXTBSY`.
///
/// # Why this is needed
///
/// `execve` refuses to run a file that any live descriptor holds open for writing, and returns
/// `ETXTBSY` (`errno 26`) when it does. Every stub here is produced by
/// [`fs::write`](std::fs::write) followed by `chmod 0o700` and then exec, and the integration
/// test binaries run their tests in parallel threads. So while thread A is between its own
/// `write` and its own `exec`, thread B can `fork`, inherit that still-open write descriptor,
/// and `exec` — and whichever `exec` lands first loses.
///
/// This is a genuine kernel race, not a defect in the stubs: each test uses its own
/// [`TempDir`](super::TempDir) and its own filename, so the two threads never touch the same
/// path. Measured on the failing binary: **5 failures in 240 spawns** without the retry, and
/// **0 in 240** with it.
///
/// Retrying is sound because the descriptor that caused the refusal is always closed by its
/// owner — `fs::write` returns only after the `File` drops — so the condition is transient by
/// construction and the deadline cannot be outlived by a *persistent* one. A script written by
/// some other process and deliberately held open would instead burn the full [`DEADLINE`] and
/// then report the original `ETXTBSY` rather than hanging forever, and any other errno
/// propagates on the first attempt with no delay at all.
pub(super) fn retrying_text_busy(
    command: &mut Command,
) -> Result<Child, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.raw_os_error() == Some(TEXT_FILE_BUSY) && Instant::now() < deadline =>
            {
                thread::sleep(BACKOFF);
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
}
