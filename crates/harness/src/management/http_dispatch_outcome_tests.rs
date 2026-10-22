// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

fn request(method: &str, path: &str) -> HttpRequest {
    HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        query: BTreeMap::new(),
        headers: BTreeMap::new(),
        body: Vec::new(),
    }
}

#[test]
fn lost_worker_uses_the_same_route_effects_as_an_expired_dispatch_wait() {
    for (method, path, class, schema) in [
        (
            "GET",
            "/v1/workflow-runs/run-1",
            "unavailable",
            "ascension.management/v1",
        ),
        (
            "GET",
            "/v1/workflow-submissions/request-1",
            "unavailable",
            "ascension.management/v1",
        ),
        (
            "POST",
            "/v1/workflow-definitions/validate",
            "unavailable",
            "ascension.management/v1",
        ),
        (
            "POST",
            "/v1/workflow-targets/preflight",
            "unavailable",
            "ascension.management/v1",
        ),
        (
            "POST",
            "/v1/workflow-runs",
            "write_state_unknown",
            "ascension.management-error/v2",
        ),
        (
            "GET",
            "/v1/future-route",
            "write_state_unknown",
            "ascension.management-error/v2",
        ),
    ] {
        let effect = route_effects::classify(&request(method, path));
        // A genuinely disconnected response channel exercises the production receive branch.
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let error = await_outcome(receiver, Instant::now() + Duration::from_secs(1), effect)
            .expect("not a panic")
            .expect_err("worker answer lost");
        assert_eq!(error.code, "dispatch_worker_lost");
        let expired = wait_failed(effect, false);
        for error in [error, expired] {
            let response = response::error_response(error, MAX_RESPONSE_BYTES).expect("wire");
            let value: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON");
            assert_eq!(response.status, 503);
            assert_eq!(value["schema_version"], schema, "{method} {path}");
            assert_eq!(value["error"]["class"], class, "{method} {path}");
        }
    }
}

#[test]
fn read_only_allowlist_does_not_admit_mutating_or_unknown_route_shapes() {
    for (method, path) in [
        ("POST", "/v1/workflow-definitions/inspect"),
        ("POST", "/v1/workflow-definitions/diff"),
        ("POST", "/v1/workflow-runs/run-1/replay"),
        ("POST", "/v1/workflow-runs/run-1/export"),
        (
            "POST",
            "/v1/workflow-runs/run-1/context-control-receipts/lookup",
        ),
        (
            "POST",
            "/v1/workflow-runs/run-1/context-owner-mutation-receipts/lookup",
        ),
        (
            "POST",
            "/v1/workflow-runs/run-1/context-owner-draft-publication-receipts/lookup",
        ),
        ("GET", "/v1/workflow-runs/run-1/events"),
        (
            "GET",
            "/v1/workflow-runs/run-1/context-owner-previews/preview-1",
        ),
    ] {
        assert_eq!(
            route_effects::classify(&request(method, path)),
            RouteEffect::ReadOnly,
            "{path}"
        );
    }
    for (method, path) in [
        ("POST", "/v1/workflow-runs/run-1/commands"),
        ("POST", "/v1/workflow-runs/run-1/context-owner-previews"),
        ("POST", "/v1/workflow-runs/run-1/context-control-commands"),
        (
            "POST",
            "/v1/workflow-runs/run-1/process-lifecycle/operations/1/reconcile",
        ),
        ("POST", "/v1/workflow-targets/preflight/execute"),
        ("DELETE", "/v1/workflow-runs/run-1"),
        ("GET", "/v1/workflow-runs/run-1/future-effect"),
        ("GET", "/v1/workflow-submissions/request-1/extra"),
    ] {
        assert_eq!(
            route_effects::classify(&request(method, path)),
            RouteEffect::MayWrite,
            "{path}"
        );
    }
    let mut paged = request("GET", "/v1/workflow-runs/run-1/events");
    paged
        .query
        .insert("after_sequence".to_owned(), "1".to_owned());
    assert_eq!(route_effects::classify(&paged), RouteEffect::ReadOnly);
    let mut preflight = request("POST", "/v1/workflow-targets/preflight");
    preflight
        .query
        .insert("future-effect".to_owned(), "true".to_owned());
    assert_eq!(route_effects::classify(&preflight), RouteEffect::MayWrite);
}

#[test]
fn only_exact_submission_recovery_can_use_the_reserved_worker_slot() {
    let mut valid = request("GET", "/v1/workflow-submissions/request-1");
    assert!(route_effects::is_submission_recovery(&valid));
    valid.query.insert("extra".to_owned(), "1".to_owned());
    assert!(!route_effects::is_submission_recovery(&valid));
    for (method, path) in [
        ("POST", "/v1/workflow-submissions/request-1"),
        ("GET", "/v1/workflow-submissions/"),
        ("GET", "/v1/workflow-submissions/request-1/extra"),
        ("GET", "/v1/workflow-runs/request-1"),
    ] {
        assert!(!route_effects::is_submission_recovery(&request(
            method, path
        )));
    }
}
