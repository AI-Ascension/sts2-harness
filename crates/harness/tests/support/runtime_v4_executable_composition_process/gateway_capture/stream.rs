// SPDX-License-Identifier: MIT

//! The low-level half of the served gateway's live stream capture: the per-pipe state, the
//! bounded drain, and the poll helpers. Refs sts2-harness#559.
//!
//! Split from `gateway_capture.rs` so neither file exceeds the repository's preferred test-file
//! size budget. The reasoning for *why* the capture exists at all — a chatty gateway used to be
//! truncated at one pipe buffer and lose its tail — is documented on `GatewayProcess`.

use std::io::{ErrorKind, Read};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::io::Errno;

/// Appended to a stream whose bytes past `MAX_CAPTURE_BYTES` were drained and dropped.
pub(crate) const TRUNCATION_NOTICE: &[u8] = b"\n{\"event\":\"gateway_output_truncated\"}\n";

/// How many bytes the drain retains from *one* stream.
///
/// #559's acceptance criterion 1 asks for a stated per-stream ceiling, and this is it. The
/// total ceiling is [`MAX_TOTAL_CAPTURE_BYTES`], which both streams share, so a gateway cannot
/// reach twice this by splitting its output across the two pipes.
pub(crate) const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;

/// How many bytes the drain retains across *both* streams together.
///
/// Without this axis the capture is bounded only per stream, so a gateway that split its output
/// evenly between stdout and stderr could still hand the evidence layer twice
/// [`MAX_CAPTURE_BYTES`]. The total is charged first, so a stream cut by the shared budget is
/// announced exactly like one cut by its own ceiling: a reader is never handed a stream that
/// looks whole when the *pair* was clipped.
pub(crate) const MAX_TOTAL_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// Reads one stream in a single drain call. Bounds a peer that keeps the descriptor readable,
/// so one drain can never monopolise the loop.
const MAX_DRAIN_READS: usize = 32;

/// Bytes one drain call may take, independent of `MAX_CAPTURE_BYTES`.
const MAX_DRAIN_BYTES: usize = 64 * 1024;

/// How long one `poll` waits before the loop re-checks the stop flag.
pub(super) const POLL_SLICE: Duration = Duration::from_millis(20);

/// How long the drain keeps polling for end of file after a stop was requested.
pub(super) const FINAL_DRAIN_GRACE: Duration = Duration::from_secs(5);

/// The bytes one drained gateway produced.
pub(super) struct Captured {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

/// The retention budget both streams share, so neither can spend what the other has not.
pub(super) struct Budget {
    remaining: usize,
}

impl Budget {
    /// Start with the whole total budget unspent.
    fn new() -> Self {
        Self {
            remaining: MAX_TOTAL_CAPTURE_BYTES,
        }
    }

    /// How many bytes may still be retained across both streams.
    fn remaining(&self) -> usize {
        self.remaining
    }

    /// Charge `count` retained bytes against the shared budget.
    ///
    /// Charged only for bytes that were actually kept, never for bytes drained and dropped, so a
    /// flood on one stream cannot starve the other stream's ceiling by spending a total it does
    /// not itself hold.
    fn charge(&mut self, count: usize) {
        self.remaining = self.remaining.saturating_sub(count);
    }
}

/// One pipe's drain state.
pub(super) struct Stream<R> {
    reader: R,
    label: &'static str,
    bytes: Vec<u8>,
    /// False once the writer has closed, so the loop can stop polling this descriptor.
    open: bool,
    /// False once polling this descriptor cannot be trusted, so the loop stops waiting on it.
    /// Separate from `open`: an undrained-but-unpollable pipe is still worth reading, it just
    /// must not hold the loop in `poll`.
    pollable: bool,
    /// False once this stream must no longer retain, after a real read error or the ceiling.
    retain: bool,
    /// The first real read error, reported at the end rather than swallowed.
    error: Option<String>,
}

impl<R> Stream<R>
where
    R: Read,
{
    fn new(reader: R, label: &'static str) -> Self {
        Self {
            reader,
            label,
            bytes: Vec::with_capacity(8192),
            open: true,
            pollable: true,
            retain: true,
            error: None,
        }
    }

    /// Take whatever is readable now, up to one call's bounds.
    ///
    /// `WouldBlock` and `Interrupted` are how a non-blocking drain says "nothing more right
    /// now"; neither is an error and neither closes the stream. A real read error closes the
    /// stream — no further reads can be trusted — but is kept for reporting rather than
    /// discarded, and retention stops so a broken capture cannot also become a large one.
    fn drain_once(&mut self, budget: &mut Budget) {
        let mut chunk = [0_u8; 8192];
        let mut reads = 0;
        let mut taken = 0;
        while reads < MAX_DRAIN_READS && taken < MAX_DRAIN_BYTES {
            reads += 1;
            match self.reader.read(&mut chunk) {
                Ok(0) => {
                    self.open = false;
                    return;
                }
                Ok(count) => {
                    taken += count;
                    self.retain(&chunk[..count], budget);
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == ErrorKind::WouldBlock => return,
                Err(error) => {
                    self.open = false;
                    self.retain = false;
                    if self.error.is_none() {
                        self.error = Some(format!(
                            "the served gateway's {} could not be read: {error} \
                             (sts2-harness#559)",
                            self.label
                        ));
                    }
                    return;
                }
            }
        }
    }

    /// Keep a read, up to the ceiling; drain and drop the rest.
    ///
    /// Two ceilings apply and being past either is the same visible fact. The tighter of the
    /// stream's own [`MAX_CAPTURE_BYTES`] and what is left of the shared
    /// [`MAX_TOTAL_CAPTURE_BYTES`] is what it keeps, and the cut is announced in one shape, so a
    /// reader never has to tell a per-stream cut from a whole-capture cut.
    fn retain(&mut self, bytes: &[u8], budget: &mut Budget) {
        if !self.retain {
            return;
        }
        let remaining = MAX_CAPTURE_BYTES
            .saturating_sub(self.bytes.len())
            .min(budget.remaining());
        if bytes.len() <= remaining {
            self.bytes.extend_from_slice(bytes);
            budget.charge(bytes.len());
            return;
        }
        self.bytes.extend_from_slice(&bytes[..remaining]);
        budget.charge(remaining);
        // The head is kept whole so a gateway's refusal and its request context survive, which
        // is what #548's attribution depends on. Only the tail beyond the ceiling is lost, and
        // it is announced rather than passed off as the whole stream.
        // The notice is cut *against the ceiling that was actually reached*, not against
        // `MAX_CAPTURE_BYTES`, so a stream stopped short by the shared total is still marked —
        // the previous form would have compared a partial buffer to a limit it never hit and
        // reported a short stream as complete.
        let reached = self.bytes.len();
        let notice_start = reached.saturating_sub(TRUNCATION_NOTICE.len());
        if self.bytes.len() > notice_start {
            self.bytes.truncate(notice_start);
        }
        self.bytes.extend_from_slice(TRUNCATION_NOTICE);
        // The notice is charged even when it trimmed bytes back off, so the bytes actually held
        // are always bounded by what the budget was charged. Without this, a stream cut short by
        // the shared total — where `reached` is below the notice's length — could hold a notice
        // the total never paid for.
        budget.charge(TRUNCATION_NOTICE.len());
        self.retain = false;
    }

    /// The stream's first real error, if it had one.
    fn error(&self) -> Option<&String> {
        self.error.as_ref()
    }
}

/// Drain both pipes until the gateway closes them, a stop is requested and its grace expires,
/// or a real read error ends them.
///
/// The two readers are separate type parameters, not one: a child hands back a `ChildStdout` and
/// a `ChildStderr`, which are different types, so a single `R` for both would never compile.
pub(super) fn drain_both<O, E>(stdout: O, stderr: E, stop: &AtomicBool) -> Result<Captured, String>
where
    O: Read + AsFd,
    E: Read + AsFd,
{
    let mut stdout = Stream::new(stdout, "stdout");
    let mut stderr = Stream::new(stderr, "stderr");
    let mut budget = Budget::new();
    let mut grace: Option<Instant> = None;
    loop {
        stdout.drain_once(&mut budget);
        stderr.drain_once(&mut budget);
        if !stdout.open && !stderr.open {
            break;
        }
        if stop.load(Ordering::Relaxed) {
            // The child is reaped by the time a stop is requested, so both pipes must reach end
            // of file. Waiting forever would be a hung test binary; giving up quietly would be
            // a silently short read, so the overrun is recorded.
            let deadline = *grace.get_or_insert_with(|| Instant::now() + FINAL_DRAIN_GRACE);
            if Instant::now() >= deadline {
                if let Some(label) = unclosed_label(&stdout, &stderr) {
                    return Err(format!(
                        "the served gateway's {label} never closed after the gateway was \
                         reaped, so its diagnostics are incomplete (sts2-harness#559)"
                    ));
                }
                break;
            }
        }
        wait_for_readable(&mut stdout, &mut stderr);
    }
    if let Some(error) = stdout.error().or_else(|| stderr.error()) {
        return Err(error.clone());
    }
    Ok(Captured {
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

/// The label of the first stream that is still open, if either is.
///
/// The two streams have different reader types, so they cannot share one homogeneous array;
/// this is the `if`-per-stream form that reports the same "never closed" fact either way.
fn unclosed_label<O, E>(stdout: &Stream<O>, stderr: &Stream<E>) -> Option<&'static str> {
    if stdout.open {
        Some(stdout.label)
    } else if stderr.open {
        Some(stderr.label)
    } else {
        None
    }
}

/// Wait until at least one open pipe is readable, bounded by [`POLL_SLICE`].
///
/// Both streams are taken mutably: a `poll` that fails for a real reason has to record the error
/// and stop trusting that descriptor, and both of those are the stream's own state.
pub(super) fn wait_for_readable<O, E>(stdout: &mut Stream<O>, stderr: &mut Stream<E>)
where
    O: Read + AsFd,
    E: Read + AsFd,
{
    // The two readers are different types (`ChildStdout` and `ChildStderr`), but `rustix::PollFd`
    // borrows any `AsFd` rather than naming the reader's type, so one poll set holds both. Each
    // `PollFd` borrows its stream's descriptor, so the set cannot outlive this function.
    let mut descriptors: Vec<PollFd<'_>> = Vec::with_capacity(2);
    if stdout.open && stdout.pollable {
        descriptors.push(PollFd::new(
            &stdout.reader,
            PollFlags::IN | PollFlags::HUP | PollFlags::ERR,
        ));
    }
    if stderr.open && stderr.pollable {
        descriptors.push(PollFd::new(
            &stderr.reader,
            PollFlags::IN | PollFlags::HUP | PollFlags::ERR,
        ));
    }
    if descriptors.is_empty() {
        // Nothing left to wait on, but a stream is still open — it was left unclosed because
        // polling it failed. Sleeping keeps that a slow drain rather than a busy wait, and the
        // recorded error still ends the loop through `drain_once` or the stop grace.
        thread::sleep(POLL_SLICE);
        return;
    }
    let slice = POLL_SLICE;
    let timeout = Timespec {
        tv_sec: slice.as_secs().min(i64::MAX as u64) as i64,
        tv_nsec: slice.subsec_nanos() as _,
    };
    match poll(&mut descriptors, Some(&timeout)) {
        Ok(_) => {}
        // A signal interrupted the wait. That is not a broken pipe, and the loop re-checks both
        // streams on the next turn, so the count is deliberately not consulted.
        Err(Errno::INTR) => {}
        Err(error) => {
            // `poll` failing for any other reason leaves the pipes undrained. The loop cannot
            // recover, and spinning on a failing poll would be a busy wait, so record it and
            // stop polling these descriptors. The next loop turn still drains whatever is
            // already buffered, then reports the failure rather than looping on it.
            //
            // `descriptors` still borrows each reader, so it has to be dropped before either
            // stream is mutated; `drop` makes that end explicit rather than relying on the
            // borrow checker to see past the end of the `match`.
            drop(descriptors);
            mark_unpollable(stdout, error);
            mark_unpollable(stderr, error);
        }
    }
}

/// Record that `stream` cannot be polled again, and why, without overwriting an earlier error.
fn mark_unpollable<R>(stream: &mut Stream<R>, error: Errno)
where
    R: Read + AsFd,
{
    if stream.error.is_none() {
        stream.error = Some(format!(
            "the served gateway's {} could not be polled: {error} (sts2-harness#559)",
            stream.label
        ));
    }
    stream.pollable = false;
}

/// Make one pipe non-blocking so a drain can never block on a full buffer.
pub(super) fn set_nonblocking<R>(reader: &R, label: &str) -> Result<(), String>
where
    R: AsFd,
{
    let flags = fcntl_getfl(reader).map_err(|error| {
        format!("the served gateway's {label} flags could not be read: {error} (sts2-harness#559)")
    })?;
    fcntl_setfl(reader, flags | OFlags::NONBLOCK).map_err(|error| {
        format!(
            "the served gateway's {label} could not be made nonblocking: {error} \
             (sts2-harness#559)"
        )
    })
}

/// Store the drain's result for [`GatewayCapture::finish`] to collect.
pub(super) fn publish(
    slot: &Mutex<Option<Result<Captured, String>>>,
    captured: Result<Captured, String>,
) {
    let Ok(mut guard) = slot.lock() else {
        // Only the drain thread writes this slot, and it writes once, so a poisoned lock means
        // this thread is unwinding. `finish` then reports the absent result rather than
        // reporting a stream that merely said nothing.
        return;
    };
    if guard.is_none() {
        *guard = Some(captured);
    }
}

/// Lock the outcome slot, treating poisoning as an absent result.
pub(super) fn lock(
    slot: &Mutex<Option<Result<Captured, String>>>,
) -> MutexGuard<'_, Option<Result<Captured, String>>> {
    slot.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
