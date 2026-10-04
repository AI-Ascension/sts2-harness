// SPDX-License-Identifier: MIT

//! Request-lifecycle reporting for the management HTTP server.
//!
//! # Why this exists
//!
//! Harness [#820](https://github.com/AI-Ascension/sts2-harness/issues/820) records the
//! decisive fact about all eight occurrences of Studio's intermittent
//! `submission_refused_502`: the owner process emits **nothing at all**. The capture
//! plumbing is proven good -- in the same artifacts `gateway-stdout.log` contains a
//! real bind line -- so the silence is the owner's own, not the harness's.
//!
//! Silence is fatal because an empty window is not evidence of anything. For every
//! occurrence three states must be told apart, and before this module all three
//! looked identical because all three produced an empty log:
//!
//! | state | what the window says |
//! | --- | --- |
//! | owner exited non-zero | nothing |
//! | owner was killed by a signal | nothing |
//! | owner stayed alive and hung | nothing |
//!
//! This module makes an empty window impossible for a served request by emitting a
//! `request_start` line and exactly one terminal line -- `request_end` when the
//! response was transmitted, `request_abandoned` when it was not -- each carrying a
//! wall-clock timestamp. A caller places an event inside a window by timestamp alone.
//!
//! **A stall or a panic is visible as itself.** A started request with no terminal
//! line is a stall or a death; a started request with a terminal line completed. The
//! absence of the terminal line *is* the signal, so the marker must never be
//! reachable without the request having really been served.
//!
//! # Redaction discipline
//!
//! The management server is a security boundary. Nothing here reads a request body,
//! a header value, a bearer credential, or a filesystem path, and nothing derived
//! from the local environment is printed. The only request-derived fields emitted
//! are the method and the route, both passed through a bounding label first:
//!
//! - **the query string is dropped** -- it is caller-controlled and can carry a
//!   secret; only the path is logged.
//! - **an unrecognisable path segment becomes `?`** -- so a secret pasted into a
//!   route, and any absolute local path, are both unrecoverable from the line.
//! - **no error message is ever logged** -- `io::Error`'s text is caller-influenced
//!   (`io_http_error` builds it with `error.to_string()`), so the abandonment marker
//!   names the *typed code* only and the server's own fixed reason.
//! - **non-printable bytes are replaced** -- a logged route must never be able to
//!   forge a second log line.
//! - **every label is byte-bounded** -- a caller cannot drive unbounded log volume.
//!
//! Every line carries the greppable prefix `sts2-management`, so a human finds it
//! without knowing this module's internals.
//!
//! # Coordination with harness #819
//!
//! #819 concerns the pre-existing `connection ended without a delivered response`
//! `eprintln!` and its missing attribution and missing assertion. This module is
//! deliberately **additive** and does not touch that line: #819's lane owns it.
//! #820's terminal `request_abandoned` marker is the same *fact* -- a response the
//! owner computed was not transmitted -- reported with per-request attribution and
//! a start marker, so the two agree and neither introduces a second report
//! mechanism for the same event.

#[path = "http_lifecycle_log_clock.rs"]
mod clock;
#[path = "http_lifecycle_log_label.rs"]
mod label;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clock::{push_decimal, push_utc_timestamp};
use label::{code_label, method_label, reason_label, route_label};

#[cfg(test)]
#[path = "http_lifecycle_log_redaction_tests.rs"]
mod redaction_tests;
#[cfg(test)]
#[path = "http_lifecycle_log_tests.rs"]
mod tests;

/// The greppable prefix every management lifecycle line carries.
pub(super) const LIFECYCLE_LOG_PREFIX: &str = "sts2-management";

/// Bound on any single caller-influenced label. One request therefore costs a fixed
/// number of bytes regardless of what the peer sent.
pub(super) const MAX_ROUTE_BYTES: usize = 128;

/// The wall-clock and monotonic clocks a sink needs.
///
/// The split is deliberate and is the reason this trait exists. Timestamps are taken
/// here, once, so they are testable: a test can assert the *value* a line carries
/// without depending on the machine's real clock. Durations come from the monotonic
/// clock only, so a wall-clock adjustment cannot produce a negative or absurd
/// duration.
pub(in crate::management::http) trait LifecycleLogSink:
    Send + Sync + std::fmt::Debug
{
    /// Seconds and nanoseconds since the UNIX epoch, or `None` if the system clock
    /// is before the epoch.
    fn wall_clock(&self) -> Option<(u64, u32)>;

    /// Milliseconds since a fixed monotonic origin, used for durations only.
    fn monotonic_millis(&self) -> u64;

    /// Write one already-redacted line. The trailing newline is the sink's concern.
    fn write_line(&self, line: &str);
}

/// The production sink: real clock, one `eprintln!`.
///
/// `eprintln!` takes the process-global stderr lock, which serialises a line against
/// other writers so two connection threads cannot interleave halves of one line.
/// That is a single lock acquisition per request on the Studio submission path,
/// which is the bounded cost the issue permits; there is deliberately no per-byte
/// work and no formatting of anything the caller sent.
#[derive(Debug)]
struct StderrLifecycleLogSink;

impl LifecycleLogSink for StderrLifecycleLogSink {
    fn wall_clock(&self) -> Option<(u64, u32)> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|since| (since.as_secs(), since.subsec_nanos().min(999_999_999)))
    }

    fn monotonic_millis(&self) -> u64 {
        // A clock before the epoch is not a reason to fail a request, so this
        // saturates instead of panicking; `wall_clock` reports the same condition
        // honestly as `ts=unavailable` in the emitted line.
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or(0);
        u64::try_from(millis).unwrap_or(u64::MAX)
    }

    fn write_line(&self, line: &str) {
        eprintln!("{line}");
    }
}

/// Monotonic sequence number shared by every request the server serves.
///
/// It is per-process and monotonic, not an identity borrowed from another namespace:
/// `request_id` in the harness belongs to a workflow request in its own namespace,
/// and reusing it here would collapse two distinct meanings into one field.
#[derive(Debug)]
struct LifecycleSequence(AtomicU64);

impl LifecycleSequence {
    fn next(&self) -> u64 {
        // `Relaxed` is correct and sufficient: the counter only has to be unique and
        // increasing, and it synchronises no other data.
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

/// The per-request lifecycle record: the attribution every line carries.
pub(in crate::management::http) struct RequestLifecycle {
    sink: Arc<dyn LifecycleLogSink>,
    request_id: u64,
    started_monotonic_millis: u64,
}

impl RequestLifecycle {
    /// Open a lifecycle record for one request. Emits nothing yet: the caller
    /// announces the start once the method and route are actually known.
    fn begin(sink: Arc<dyn LifecycleLogSink>, sequence: &Arc<LifecycleSequence>) -> Self {
        let started_monotonic_millis = sink.monotonic_millis();
        Self {
            sink,
            request_id: sequence.next(),
            started_monotonic_millis,
        }
    }

    /// The id this request's lines are correlated by.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Emit the start marker. Both fields pass through a bounding label, so neither
    /// can carry caller-controlled bytes into the log.
    pub(in crate::management::http) fn log_start(&self, method: &str, route: &str) {
        self.emit(
            "request_start",
            &[method_label(method), route_label(route)],
            None,
        );
    }

    /// Emit the terminal marker for a connection that ended **without a readable
    /// request**.
    ///
    /// A request that never parsed has no method and no route to name, so this takes
    /// no attribution fields at all. That is deliberate, and it is why this marker
    /// exists rather than letting the `request_end` path cover the case: a bare
    /// `request_end` with no `request_start` and no route is a line nobody can act on,
    /// which is precisely the unattributed-report defect harness #819 documents. A
    /// request that never arrived is a state an operator must be able to see and
    /// distinguish from one that arrived and was refused, so it gets its own marker
    /// and the bounded, harness-owned literal below says so.
    ///
    /// Only the typed `code` is logged, for the same reason
    /// [`Self::log_abandoned`] logs only the code: `HttpError::message` is built from
    /// an `io::Error` whose text can embed caller bytes.
    pub(in crate::management::http) fn log_unreadable(&self, code: &str) {
        self.emit(
            "request_unreadable",
            &[code_label(code), reason_label("no request was read")],
            None,
        );
    }

    /// Emit the terminal marker for a request whose response was transmitted.
    pub(in crate::management::http) fn log_end(&self, status: u16) {
        self.emit("request_end", &[], Some(status));
    }

    /// Emit the terminal marker for a request whose computed response was **not**
    /// transmitted. This is the state that used to be indistinguishable from a hang.
    ///
    /// Takes only the typed `code` and **no message**. `HttpError::message` is not
    /// trustworthy as log input: `io_http_error` builds it from `io::Error`, whose
    /// text can embed caller-supplied bytes, and logging it here would be exactly
    /// the leak this module exists to prevent. The code is a harness-owned constant
    /// and is sufficient to tell the states apart.
    pub(in crate::management::http) fn log_abandoned(&self, code: &str) {
        self.emit(
            "request_abandoned",
            &[code_label(code), reason_label(code)],
            None,
        );
    }

    /// Emit the terminal marker for a request whose handler **panicked**.
    ///
    /// This is the third way a started request can fail to finish. Without its own
    /// marker a panic and a hang would both appear as a start line with no terminal
    /// line, which is the one distinction this log exists to make: a hang is a live
    /// thread that never returned, and a panic is a thread that died. The code is a
    /// harness-owned literal, and no panic payload is ever read -- a panic message can
    /// carry caller bytes, so it is the same redaction rule `log_abandoned` follows.
    pub(in crate::management::http) fn log_panicked(&self) {
        self.emit(
            "request_panicked",
            &[
                code_label("request_panicked"),
                reason_label("request_panicked"),
            ],
            None,
        );
    }

    fn emit(&self, marker: &str, fields: &[String], status: Option<u16>) {
        let mut line = String::with_capacity(160);
        line.push_str(LIFECYCLE_LOG_PREFIX);
        line.push(' ');
        line.push_str(marker);
        // One clock read, used for both the human timestamp and the epoch field, so
        // the two can never disagree.
        match self.sink.wall_clock() {
            Some((seconds, nanos)) => {
                line.push_str(" ts=");
                push_utc_timestamp(&mut line, seconds, nanos);
                line.push_str(" epoch_secs=");
                push_decimal(&mut line, seconds);
            }
            // Honest rather than fabricated: a clock before the epoch reports an
            // explicit `ts=unavailable` rather than inventing a plausible time.
            None => line.push_str(" ts=unavailable epoch_secs=-"),
        }
        line.push_str(" request_id=");
        push_decimal(&mut line, self.request_id);
        if let Some(status) = status {
            line.push_str(" status=");
            push_decimal(&mut line, u64::from(status));
        }
        for field in fields {
            line.push(' ');
            line.push_str(field);
        }
        line.push_str(" elapsed_ms=");
        push_decimal(
            &mut line,
            self.sink
                .monotonic_millis()
                .saturating_sub(self.started_monotonic_millis),
        );
        self.sink.write_line(&line);
    }
}

impl std::fmt::Debug for RequestLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestLifecycle")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

/// The lifecycle record source for one server: a sink plus its sequence.
#[derive(Clone, Debug)]
pub(in crate::management::http) struct RequestLifecycleLog {
    sink: Arc<dyn LifecycleLogSink>,
    sequence: Arc<LifecycleSequence>,
}

impl Default for RequestLifecycleLog {
    fn default() -> Self {
        Self::stderr()
    }
}

impl RequestLifecycleLog {
    /// The production record source: real clock, real stderr, per-process sequence.
    pub(super) fn stderr() -> Self {
        Self {
            sink: Arc::new(StderrLifecycleLogSink),
            sequence: Arc::new(LifecycleSequence(AtomicU64::new(0))),
        }
    }

    /// A record source over an injected sink. Used by the tests to assert the exact
    /// line a request emits, which is what makes the emission mutation-provable.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn with_sink(sink: Arc<dyn LifecycleLogSink>) -> Self {
        Self {
            sink,
            sequence: Arc::new(LifecycleSequence(AtomicU64::new(0))),
        }
    }

    /// Begin one request's lifecycle record.
    pub(in crate::management::http) fn begin(&self) -> RequestLifecycle {
        RequestLifecycle::begin(Arc::clone(&self.sink), &self.sequence)
    }
}
