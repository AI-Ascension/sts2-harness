// SPDX-License-Identifier: MIT

//! Live capture of the served gateway's own streams. Refs sts2-harness#559.
//!
//! The served compositions spawn the gateway with piped stdout/stderr
//! (`runtime_v4_executable_composition_process.rs`) and, until this module, read neither pipe
//! until `stop` SIGKILLed the process group and called `wait_with_output()`. That was the *only*
//! moment either pipe was ever read. A pipe holds one buffer — 65,536 bytes on Linux — before
//! the writer blocks, so a gateway that wrote more than one buffer was still blocked mid-write
//! when the group was killed, and everything it had not yet written was discarded: no error, no
//! truncation notice, no diagnostic. A chatty gateway was clipped at exactly one pipe buffer
//! and lost its own tail, which is the same defect class as #541 and #548 — a served path left
//! holding nothing that explains why.
//!
//! Bounding at the writer does not repair this. The in-memory `Output` is already unbounded
//! before any write-side ceiling applies, so a cap on the write is downstream of the bytes this
//! module recovers. The bound belongs here, on the capture, and the bytes have to be taken
//! **while the child runs**.
//!
//! ## Mechanism
//!
//! This follows `completed_resume_process_support.rs`, the precedent already in this crate:
//! non-blocking descriptors so a drain can never block, a `poll` loop so it costs nothing while
//! the gateway is quiet, a per-call read cap and a per-stream retention ceiling so the drain
//! cannot itself become an unbounded path, and a bounded grace so the drain can never outlive
//! the process it is draining.
//!
//! The one structural difference from the precedent is *who* drives the loop. Every child there
//! runs under one fixed timeout, so its caller already owns a poll loop and can drain inside
//! it. The served gateway does not: after `ready` returns, the gateway serves for as long as the
//! scenario takes and only `stop` ends it, and the scenario body is arbitrary code this module
//! cannot instrument. So the loop is driven by a single thread that starts at spawn, drains
//! both pipes for the whole life of the gateway, and is joined only by `stop`. No scenario can
//! forget to pump it, because the pump is not in the scenario.
//!
//! One thread owns both pipes rather than one per pipe so there is a single owner to stop and
//! join, and a single place where a real read error is recorded.
//!
//! ## Error policy
//!
//! Every exit path is deliberate:
//!
//! * `EAGAIN` and `EINTR` are the normal outcomes of a non-blocking drain and are not errors.
//! * A real read error is recorded and reported, and that stream stops being polled: retrying
//!   a descriptor that has just failed turns one real error into a busy loop. A gateway can
//!   then block on the unread pipe, but it cannot block *forever* — every scenario ends by
//!   `stop`, which SIGKILLs the group, and the whole served path has a deadline above that.
//!   The failure is reported rather than left to be discovered as a truncated stream.
//! * Exceeding the retention ceiling is *not* an error. Bytes past the ceiling are drained and
//!   dropped, and a machine-readable notice is appended, so a reader can tell a short stream
//!   from a clipped one — the exact ambiguity this issue is about.
//! * A reader that has not reached end of file after the child was reaped is reported, not
//!   waited on. A reaped child cannot hold a write end open, so an overrun is a defect, and a
//!   drain that outlives its stop grace is reported rather than joined, so a broken capture can
//!   never become a hung test binary.

use std::process::{Child, ChildStderr, ChildStdout, ExitStatus, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The per-pipe drain mechanics live in a sibling module so neither file exceeds the
/// repository's preferred test-file size budget.
#[path = "gateway_capture/stream.rs"]
pub(crate) mod stream;

use self::stream::{Captured, FINAL_DRAIN_GRACE, drain_both, lock, publish, set_nonblocking};

///
/// `ready` and `stop` take this type so every existing call site keeps its current shape and
/// the #548 contract — a served failure still receives a complete `Output` — is unchanged.
pub(crate) struct GatewayProcess {
    child: Child,
    capture: Option<GatewayCapture>,
    /// True once the caller has reaped the child and is deliberately winding up, so the `Drop`
    /// below does not also signal a process that is already gone.
    finished: bool,
}

impl GatewayProcess {
    /// Take both pipes and start draining them.
    ///
    /// Called immediately after spawn, before anything can wait on the gateway, so neither pipe
    /// is ever unread while the child runs. A pipe that was not piped is reported with the
    /// stream it names.
    ///
    /// Every failure below kills *and reaps* the child before it propagates. Dropping a `Child`
    /// does not stop the process — it only closes the handles — so a plain `?` here would return
    /// the error and leave a live gateway behind, still holding the address the scenario was
    /// about to use. `abort` is what makes the comment at the spawn site true.
    pub(crate) fn attach(child: Child) -> Result<Self, String> {
        let mut child = child;
        let stdout = child.stdout.take().ok_or_else(|| {
            abort(
                &mut child,
                "the served gateway exposed no stdout, so its diagnostics cannot be captured \
                 (sts2-harness#559)",
            )
        })?;
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                // `stdout` drops on the way out, closing this read end. Nothing was drained from
                // it because the drain does not start until both pipes are in hand.
                return Err(abort(
                    &mut child,
                    "the served gateway exposed no stderr, so its diagnostics cannot be \
                     captured (sts2-harness#559)",
                ));
            }
        };
        match GatewayCapture::start(stdout, stderr) {
            Ok(capture) => Ok(Self {
                capture: Some(capture),
                child,
                finished: false,
            }),
            Err(error) => Err(abort(&mut child, &error)),
        }
    }

    /// Fail early if the capture has already broken, so a scenario reports the real defect
    /// instead of a downstream symptom.
    pub(super) fn check_capture(&self) -> Result<(), String> {
        if let Some(error) = self.capture.as_ref().and_then(GatewayCapture::peek_error) {
            return Err(error);
        }
        Ok(())
    }

    /// Reap the gateway without collecting its streams, reporting whether it had already
    /// exited. Callers use this to notice a gateway that died before its scenario reached it.
    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// The gateway's process id, used to signal its whole group.
    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }

    /// Kill the gateway itself, for the case where its group could not be signalled.
    pub(super) fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    /// Reap the gateway and return its exit status.
    ///
    /// This is what closes the gateway's end of both pipes, so the drain reaches end of file.
    /// `Child::wait_with_output` is deliberately not offered: it reads the pipes itself, and
    /// these bytes are already being read by the capture.
    pub(super) fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.child.wait()
    }

    /// Stop the capture and build the `Output` the served sites already expect.
    pub(super) fn finish(mut self, status: ExitStatus) -> Result<Output, String> {
        // The caller reaps the child before reaching this, so the `Drop` below would find a
        // process that has already exited and do nothing. Setting the flag says so explicitly
        // rather than relying on that, so the deliberate path is not also the abort path.
        self.finished = true;
        // `capture` is an `Option` because this type implements `Drop`, and a `Drop` type cannot
        // move a field out of itself. Taking the capture here hands it to `finish`, and the
        // `Drop` then sees `None` and has nothing left to wind up.
        match self.capture.take() {
            Some(capture) => capture.finish(status),
            None => Err(String::from(
                "the served gateway's capture was already taken, so its diagnostics are \
                 unavailable (sts2-harness#559)",
            )),
        }
    }
}

impl Drop for GatewayProcess {
    /// Kill and reap a gateway whose scenario returned early.
    ///
    /// A served scenario returns through `?` in a number of places — a failed `ready`, a failed
    /// request, a failed assertion — and each of those drops its `GatewayProcess` here. Without
    /// this the gateway would keep serving on its address after the test had moved on, and its
    /// capture thread would keep polling a pipe whose writer never closes. The capture drops with
    /// this and sets its stop flag, so the thread winds up on its own bounded grace instead of
    /// being left to poll a pipe no scenario will ever close.
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .status();
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Kill and reap `child`, then return `reason` so a setup failure can propagate.
///
/// The group signal is the same one `stop` uses, because the gateway is spawned with
/// `process_group(0)` and may itself have children. `kill` is best-effort: the reason the abort
/// happened is reported either way, and a failure to reap is appended to it rather than replacing
/// it, so a leaked process is visible in the failure instead of being silently lost.
fn abort(child: &mut Child, reason: &str) -> String {
    let mut problems = Vec::new();
    let _group = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .status();
    if let Err(error) = child.kill() {
        // The group kill above usually wins, and killing an already-dead child is the expected
        // outcome here, so this is only recorded rather than raised.
        if !matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
        ) {
            problems.push(format!("the served gateway could not be killed: {error}"));
        }
    }
    // Reaping is not optional: a killed-but-unreaped child is a zombie held for the life of the
    // test binary, and the failure to reap is appended to the reason so it stays visible.
    if let Err(error) = child.wait() {
        problems.push(format!("the served gateway could not be reaped: {error}"));
    }
    if problems.is_empty() {
        reason.to_owned()
    } else {
        format!("{reason}; {}", problems.join("; "))
    }
}

/// The background drain for one gateway's two pipes.
struct GatewayCapture {
    drain: Option<thread::JoinHandle<()>>,
    outcome: Arc<Mutex<Option<Result<Captured, String>>>>,
    stop: Arc<AtomicBool>,
}

impl GatewayCapture {
    /// Make both pipes non-blocking and start the single thread that drains them.
    fn start(stdout: ChildStdout, stderr: ChildStderr) -> Result<Self, String> {
        set_nonblocking(&stdout, "stdout")?;
        set_nonblocking(&stderr, "stderr")?;
        let outcome: Arc<Mutex<Option<Result<Captured, String>>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_outcome = Arc::clone(&outcome);
        let worker_stop = Arc::clone(&stop);
        let drain = thread::Builder::new()
            .name(String::from("sts2-gateway-capture"))
            .spawn(move || {
                let captured = drain_both(stdout, stderr, &worker_stop);
                publish(&worker_outcome, captured);
            })
            .map_err(|error| {
                format!("the served gateway's capture could not start: {error} (sts2-harness#559)")
            })?;
        Ok(Self {
            drain: Some(drain),
            outcome,
            stop,
        })
    }

    /// The capture's first real error, if it has already finished with one.
    fn peek_error(&self) -> Option<String> {
        match &*lock(&self.outcome) {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        }
    }

    /// Ask the drain to wind up, then wait a bounded grace and build the `Output`.
    ///
    /// The caller must have reaped the child first. That is what makes end of file certain: the
    /// write end of each pipe is held only by the child, and a reaped child holds nothing, so
    /// the drain finishes on its own. The grace bounds that wait instead of assuming it.
    fn finish(mut self, status: ExitStatus) -> Result<Output, String> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(drain) = self.drain.take() {
            let deadline = Instant::now() + FINAL_DRAIN_GRACE;
            while !drain.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if !drain.is_finished() {
                return Err(String::from(
                    "the served gateway's capture did not finish after the gateway was reaped, \
                     so its diagnostics are incomplete (sts2-harness#559)",
                ));
            }
            // A reader that panics has stored nothing, which is reported below rather than
            // being passed off as a stream that simply said nothing.
            let _ = drain.join();
        }
        match lock(&self.outcome).take() {
            Some(Ok(captured)) => Ok(Output {
                status,
                stdout: captured.stdout,
                stderr: captured.stderr,
            }),
            Some(Err(error)) => Err(error),
            None => Err(String::from(
                "the served gateway's capture produced no result, so its diagnostics are \
                 unavailable (sts2-harness#559)",
            )),
        }
    }
}

impl Drop for GatewayCapture {
    fn drop(&mut self) {
        // An abandoned capture must not leave a thread polling a descriptor nobody will close.
        // The flag bounds it; the thread is left to wind up on its own.
        self.stop.store(true, Ordering::Relaxed);
    }
}
