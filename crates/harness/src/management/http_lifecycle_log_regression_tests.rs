// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use super::super::{LifecycleLogSink, StderrLifecycleLogSink};
use super::{RecordingSink, log_with};

#[test]
fn forward_and_backward_wall_clock_steps_do_not_change_elapsed_duration() {
    for (start_seconds, end_seconds) in [(10_000, 3_610_000), (3_610_000, 10_000)] {
        let sink = Arc::new(RecordingSink::with_clock_script(
            vec![Some((start_seconds, 0)), Some((end_seconds, 0))],
            vec![5_000, 5_042],
        ));
        let lifecycle = log_with(&sink);
        let request = lifecycle.begin();
        request.log_start("GET", "/v1/health");
        request.log_end(200);

        let lines = sink.lines();
        assert!(
            lines[0].contains(&format!("epoch_secs={start_seconds}")),
            "start keeps its wall-clock timestamp: {:?}",
            lines[0]
        );
        assert!(
            lines[1].contains(&format!("epoch_secs={end_seconds}")),
            "end keeps its wall-clock timestamp: {:?}",
            lines[1]
        );
        assert!(
            lines[1].contains("elapsed_ms=42"),
            "elapsed duration must come from the independent monotonic clock: {:?}",
            lines[1]
        );
    }
}

#[test]
fn production_monotonic_clock_is_relative_not_unix_epoch_time() {
    let sink = StderrLifecycleLogSink;
    let (wall_seconds, wall_nanos) = sink
        .wall_clock()
        .expect("the test machine wall clock must be after the UNIX epoch");
    let wall_millis = u128::from(wall_seconds) * 1_000 + u128::from(wall_nanos / 1_000_000);
    let monotonic_millis = u128::from(sink.monotonic_millis());

    assert!(
        monotonic_millis < wall_millis / 2,
        "production elapsed time must be relative to an Instant origin, not UNIX time: \
         monotonic={monotonic_millis}, wall={wall_millis}"
    );
}

#[test]
fn served_route_templates_keep_static_steps_and_redact_dynamic_identifiers() {
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0]));
    let lifecycle = log_with(&sink);
    let cases = [
        (
            "POST",
            "/v1/workflow-definitions/validate",
            "route=/v1/workflow-definitions/validate",
        ),
        (
            "POST",
            "/v1/workflow-definitions/inspect",
            "route=/v1/workflow-definitions/inspect",
        ),
        (
            "POST",
            "/v1/workflow-definitions/diff",
            "route=/v1/workflow-definitions/diff",
        ),
        (
            "POST",
            "/v1/inference-profiles/validate/revisions",
            "route=/v1/inference-profiles/{profile_id}/revisions",
        ),
        (
            "GET",
            "/v1/studio/authoring-inference/operations/inspect/diff",
            "route=/v1/studio/authoring-inference/operations/{draft_id}/{client_mutation_id}",
        ),
        (
            "GET",
            "/v1/workflow-runs/health/events?token=sk-test-secret",
            "route=/v1/workflow-runs/{run_id}/events",
        ),
    ];

    for (method, path, expected_route) in cases {
        lifecycle.begin().log_start(method, path);
        let line = sink
            .lines()
            .pop()
            .expect("each request emits its start line");
        let route = line
            .split_whitespace()
            .find(|field| field.starts_with("route="))
            .expect("every start line carries a route label");
        assert_eq!(route, expected_route, "route for {method} {path}");
        assert!(
            route.len() <= super::super::MAX_ROUTE_BYTES,
            "route label is bounded: {route}"
        );
        assert!(
            !line.contains("sk-test-secret"),
            "query values must not reach a route label: {line}"
        );
    }
}

#[test]
fn unmatched_paths_do_not_whitelist_caller_ids_that_look_like_route_words() {
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0]));
    let lifecycle = log_with(&sink);
    let request = lifecycle.begin();
    request.log_start("GET", "/v1/workflow-runs/health/unserved?token=sk-private");

    let line = sink
        .lines()
        .pop()
        .expect("the request emits its start line");
    assert!(line.contains("route=unmatched"), "got {line:?}");
    assert!(
        !line.contains("health") && !line.contains("unserved") && !line.contains("sk-private"),
        "an unmatched path must not whitelist caller-controlled segments: {line:?}"
    );
}
