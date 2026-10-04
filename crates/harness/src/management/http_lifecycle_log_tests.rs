// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

//! # The defect these guard
//!
//! Harness [#820](https://github.com/AI-Ascension/sts2-harness/issues/820): across
//! eight occurrences of Studio's `submission_refused_502`, the owner process wrote
//! **nothing at all**. `owner-stdout.log` was 0 bytes and `owner-stderr.log` held
//! nothing but the Studio fixture's own two lifecycle lines.
//!
//! The property under test is therefore not "the server logs something". It is the
//! pair of claims that made the silence fatal:
//!
//! 1. **Every served request is attributable, and its terminal state is marked.**
//!    A start line with a terminal line means it completed. A start line with *no*
//!    terminal line is the stall or the death -- and that absence is the whole
//!    signal, so it must be reachable only when the request really was served.
//! 2. **The reporting cannot leak.** A token, an `Authorization` value, a body, or
//!    an absolute local path must never reach the log, however it arrived.
//!
//! ## Why these tests are structured against the sink
//!
//! Both claims are asserted against the *exact emitted lines* through an injected
//! sink, not against "stderr is non-empty". That distinction is the point of #820's
//! mutation requirement and of #819's finding: a test that passes with the emission
//! deleted is worse than no test at all. Because every assertion here reads lines
//! that only `log_start` / `log_end` / `log_abandoned` can produce, deleting an
//! emission makes the matching assertion fail -- proven by mutation in the PR body.
//!
//! ## Why the timestamps are injected
//!
//! The lines are asserted for their *value*, including the rendered UTC timestamp,
//! so a test can prove an event is placeable in a window by timestamp alone. That
//! needs a fixed clock: asserting against the machine's real clock would prove only
//! that some number was printed. `FixedClock` supplies exact epoch seconds, and the
//! elapsed-milliseconds assertions use a scripted clock rather than real sleeping,
//! so no assertion in this file depends on wall-clock timing and a slow or contended
//! runner cannot flip a result.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{LifecycleLogSink, RequestLifecycleLog};

/// A sink that records every line and serves a fixed, scripted clock.
#[derive(Debug)]
pub(super) struct RecordingSink {
    lines: Mutex<Vec<String>>,
    /// Epoch seconds returned by `wall_clock`.
    epoch_seconds: Option<u64>,
    /// Nanoseconds returned by `wall_clock`.
    epoch_nanos: u32,
    /// Values `monotonic_millis` returns, in order; the last one repeats forever so
    /// a request that reads the clock more than once still terminates.
    monotonic: Vec<u64>,
    monotonic_index: AtomicU64,
}

impl RecordingSink {
    pub(super) fn new(epoch_seconds: u64, monotonic: Vec<u64>) -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
            epoch_seconds: Some(epoch_seconds),
            epoch_nanos: 123_000_000,
            monotonic,
            monotonic_index: AtomicU64::new(0),
        }
    }

    /// A sink whose wall clock is before the UNIX epoch.
    pub(super) fn with_absent_clock(monotonic: Vec<u64>) -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
            epoch_seconds: None,
            epoch_nanos: 0,
            monotonic,
            monotonic_index: AtomicU64::new(0),
        }
    }

    pub(super) fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .expect("the sink lock must be poison-free")
            .clone()
    }

    pub(super) fn joined(&self) -> String {
        self.lines().join("\n")
    }
}

impl LifecycleLogSink for RecordingSink {
    fn wall_clock(&self) -> Option<(u64, u32)> {
        self.epoch_seconds
            .map(|seconds| (seconds, self.epoch_nanos))
    }

    fn monotonic_millis(&self) -> u64 {
        let index =
            usize::try_from(self.monotonic_index.fetch_add(1, Ordering::Relaxed)).unwrap_or(0);
        // The final scripted value repeats, so a request that reads the clock more
        // often than the script anticipated still gets a deterministic answer.
        self.monotonic
            .get(index)
            .copied()
            .or_else(|| self.monotonic.last().copied())
            .unwrap_or(0)
    }

    fn write_line(&self, line: &str) {
        self.lines
            .lock()
            .expect("the sink lock must be poison-free")
            .push(line.to_owned());
    }
}

pub(super) fn log_with(sink: &Arc<RecordingSink>) -> RequestLifecycleLog {
    RequestLifecycleLog::with_sink(Arc::clone(sink) as Arc<dyn LifecycleLogSink>)
}

// ## 1. Observability: every served request is attributable and terminal

#[test]
fn a_served_request_emits_a_start_and_a_terminal_line() {
    // The core claim. Before #820 this produced nothing at all.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![1_000, 1_008]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start("POST", "/v1/workflow-runs");
    request.log_end(200);

    let lines = sink.lines();
    assert_eq!(
        lines.len(),
        2,
        "a served request must emit exactly a start and a terminal line, got {lines:?}"
    );
    assert!(
        lines[0].starts_with("sts2-management request_start "),
        "the first line must mark the start and be greppable, got {:?}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("sts2-management request_end "),
        "the second line must mark completion, got {:?}",
        lines[1]
    );
    assert!(
        lines[0].contains("method=POST") && lines[0].contains("route=/v1/workflow-runs"),
        "the start line must name the method and route, got {:?}",
        lines[0]
    );
    assert!(
        lines[1].contains("status=200"),
        "the terminal line must name the status the peer was told, got {:?}",
        lines[1]
    );
}

#[test]
fn an_unreadable_request_is_named_by_its_own_marker() {
    // A connection that never delivered a readable request has no method and no route,
    // so it cannot emit a start line. What it must not do is emit a terminal line that
    // claims an ordinary completion for a request that never arrived: the resulting
    // line is unattributable, which is the defect class #819 documents.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![1_000, 1_040]));
    let lifecycle = log_with(&sink);

    let connection = lifecycle.begin();
    connection.log_unreadable("deadline_exceeded");

    let lines = sink.lines();
    assert_eq!(
        lines.len(),
        1,
        "an unreadable request emits exactly one terminal line, got {lines:?}"
    );
    assert!(
        lines[0].starts_with("sts2-management request_unreadable "),
        "the line must name its own condition rather than borrow `request_end`, got {:?}",
        lines[0]
    );
    assert!(
        lines[0].contains("code=deadline_exceeded"),
        "the line must name the typed code that stopped the read, got {:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains("status="),
        "no status may be claimed: no response was delivered for a request that never \
         arrived, got {:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains("request_start") && !lines[0].contains("route="),
        "a line with no start line before it must not invent a route, got {:?}",
        lines[0]
    );
}

#[test]
fn every_marker_line_carries_a_timestamp_placing_it_in_a_window() {
    // Requirement: a caller places an event inside a window *by timestamp alone*.
    // Both markers must therefore carry a rendered timestamp, not just an id.
    let sink = Arc::new(RecordingSink::new(1_767_225_224, vec![5_000, 5_042]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start("GET", "/v1/health");
    request.log_end(200);

    let lines = sink.lines();
    for line in &lines {
        assert!(
            line.contains("ts=2025-12-31T23:53:44.123Z"),
            "every marker line must carry the rendered UTC timestamp, got {line:?}"
        );
        assert!(
            line.contains("epoch_secs=1767225224"),
            "every marker line must carry a machine-comparable epoch, got {line:?}"
        );
    }
    // The elapsed time is monotonic-sourced, so it is asserted exactly: 42ms here.
    assert!(
        lines[1].contains("elapsed_ms=42"),
        "the terminal line must carry the monotonic elapsed time, got {:?}",
        lines[1]
    );
}

#[test]
fn a_clock_before_the_epoch_reports_unavailable_rather_than_a_fabricated_time() {
    // An empty or invented timestamp is worse than an honest one: it would let a
    // reader place an event in the wrong window with total confidence.
    let sink = Arc::new(RecordingSink::with_absent_clock(vec![0, 1]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start("GET", "/v1/health");
    request.log_end(200);

    let joined = sink.joined();
    assert!(
        joined.contains("ts=unavailable"),
        "an unavailable clock must be reported honestly, got {joined:?}"
    );
    assert!(
        !joined.contains("1970-01-01"),
        "a clock before the epoch must not be rendered as the epoch itself, got {joined:?}"
    );
}

#[test]
fn an_untransmitted_response_is_marked_abandoned_not_completed() {
    // The state #820 exists for. Before it, "the peer never got the answer" was
    // the same empty window as a clean exit and a hang.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![10, 8_460]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start("POST", "/v1/workflow-runs");
    request.log_abandoned("deadline_exceeded");

    let lines = sink.lines();
    assert_eq!(lines.len(), 2, "got {lines:?}");
    assert!(
        lines[1].starts_with("sts2-management request_abandoned "),
        "a response that was not transmitted must be marked abandoned, got {:?}",
        lines[1]
    );
    assert!(
        !lines[1].contains("request_end"),
        "an abandoned request must never also be marked as ended, got {:?}",
        lines[1]
    );
    assert!(
        lines[1].contains("code=deadline_exceeded"),
        "the abandonment must name the typed code, got {:?}",
        lines[1]
    );
    assert!(
        lines[1].contains("elapsed_ms=8450"),
        "an 8.4s stall must be visible as itself in the elapsed time, got {:?}",
        lines[1]
    );
}

#[test]
fn a_started_request_with_no_terminal_line_is_the_stall_signal() {
    // The negative space is the product. A start with no terminal line is exactly
    // "this request did not finish" -- a hang or a death of the connection thread.
    //
    // This asserts the property by showing that the *only* thing separating the two
    // states is the terminal emission: the start line is byte-identical either way.
    let completed_sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));
    let stalled_sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));

    let completed = log_with(&completed_sink).begin();
    completed.log_start("POST", "/v1/workflow-runs");
    completed.log_end(200);

    let stalled = log_with(&stalled_sink).begin();
    stalled.log_start("POST", "/v1/workflow-runs");
    // No terminal marker: the connection thread died or blocked here.

    let completed_lines = completed_sink.lines();
    let stalled_lines = stalled_sink.lines();
    assert_eq!(
        completed_lines[0], stalled_lines[0],
        "the start line is identical, so the terminal line is the only discriminator"
    );
    assert_eq!(
        completed_lines.len(),
        2,
        "a completed request has a terminal line"
    );
    assert_eq!(
        stalled_lines.len(),
        1,
        "a stalled request is distinguishable *only* by the absent terminal line, \
         which is why that emission must never be deleted"
    );
}

#[test]
fn the_request_id_correlates_the_two_lines_of_one_request() {
    // Attribution: which connection, when several requests interleave.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1, 2, 3]));
    let lifecycle = log_with(&sink);

    let first = lifecycle.begin();
    first.log_start("GET", "/v1/health");
    first.log_end(200);
    let second = lifecycle.begin();
    second.log_start("POST", "/v1/workflow-runs");
    second.log_end(202);

    let lines = sink.lines();
    let id_of = |line: &str| {
        line.split_whitespace()
            .find_map(|field| field.strip_prefix("request_id="))
            .map(str::to_owned)
            .expect("every line must carry a request_id")
    };
    assert_eq!(
        id_of(&lines[0]),
        id_of(&lines[1]),
        "both lines of one request must share its id"
    );
    assert_eq!(
        id_of(&lines[2]),
        id_of(&lines[3]),
        "both lines of the second request must share its id"
    );
    assert_ne!(
        id_of(&lines[0]),
        id_of(&lines[2]),
        "two requests must not share an id"
    );
    assert_ne!(
        first.request_id(),
        second.request_id(),
        "the ids must come from one monotonic sequence"
    );
}

#[test]
fn the_cost_of_a_request_is_bounded_regardless_of_what_the_peer_sends() {
    // Bounded cost: an unbounded route is clamped, so a caller cannot drive log
    // volume, and cannot use the log as a side channel for a long payload.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));
    let lifecycle = log_with(&sink);

    // A route made entirely of known segments, so the clamp -- not the vocabulary
    // check -- is what bounds it. This is the worst case for line length.
    let hostile_route = format!("/v1/{}", "health/".repeat(2_048));
    let request = lifecycle.begin();
    request.log_start("GET", &hostile_route);
    request.log_end(200);

    let lines = sink.lines();
    for line in &lines {
        assert!(
            line.len() < 512,
            "a lifecycle line must stay bounded, got {} bytes: {line:?}",
            line.len()
        );
    }
    assert!(
        lines[0].contains("..truncated"),
        "an over-long route must be explicitly marked as truncated, got {:?}",
        lines[0]
    );
    // The unbounded input must not be echoed even in truncated form.
    assert!(
        !lines[0].contains(&"a".repeat(64)),
        "the raw payload must never be echoed, got {:?}",
        lines[0]
    );
}

#[test]
fn an_unrecognised_route_segment_is_replaced_so_the_log_stays_useful() {
    // The redaction property and the observability property must coexist: a route the
    // server does not serve is redacted, a route it does serve is readable.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));
    let lifecycle = log_with(&sink);

    let served = lifecycle.begin();
    served.log_start("POST", "/v1/workflow-runs");
    served.log_end(200);

    let unknown = lifecycle.begin();
    unknown.log_start("GET", "/v1/not-a-real-route/hunter2");
    unknown.log_end(404);

    let lines = sink.lines();
    assert!(
        lines[0].contains("route=/v1/workflow-runs"),
        "a served route must stay readable, got {:?}",
        lines[0]
    );
    assert!(
        lines[2].contains("route=/v1/?/?"),
        "an unserved route must be reported structurally, not echoed, got {:?}",
        lines[2]
    );
    assert!(
        !lines[2].contains("hunter2"),
        "an unrecognised segment must not be echoed, got {:?}",
        lines[2]
    );
}
