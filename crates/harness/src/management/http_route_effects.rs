// SPDX-License-Identifier: MIT

//! Effects belong to admitted routes, not HTTP methods. Unknown shapes stay conservative.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RouteEffect {
    ReadOnly,
    MayWrite,
}

pub(super) fn classify(request: &HttpRequest) -> RouteEffect {
    let segments = request.path.split('/').collect::<Vec<_>>();
    let route = (request.method.as_str(), segments.as_slice());
    // These handlers either ignore the query or explicitly validate their read-only pagination.
    if matches!(
        route,
        ("GET", ["", "v1", "health"])
            | (
                "POST",
                [
                    "",
                    "v1",
                    "workflow-definitions",
                    "validate" | "inspect" | "diff"
                ]
            )
            | (
                "GET",
                [
                    "",
                    "v1",
                    "workflow-runs",
                    _,
                    "events" | "context-owner-items" | "context-owner-revisions"
                ]
            )
    ) {
        return RouteEffect::ReadOnly;
    }
    if !request.query.is_empty() {
        return RouteEffect::MayWrite;
    }
    if matches!(
        route,
        (
            "GET",
            [
                "",
                "v1",
                "workflow-targets"
                    | "context-bindings"
                    | "inference-profiles"
                    | "capabilities"
                    | "memory-policy-owner"
            ]
        ) | ("GET", ["", "v2", "workflow-targets"])
            | ("GET", ["", "v1", "workflow-submissions", _])
            | ("GET", ["", "v2", "workflow-runs", _, "seed-binding"])
            | ("GET", ["", "v1", "studio", "definitions"])
            | ("GET", ["", "v1", "studio", "drafts", _])
            | (
                "GET",
                [
                    "",
                    "v1",
                    "studio",
                    "authoring-inference",
                    "operations",
                    _,
                    _
                ]
            )
            | (
                "GET",
                ["", "v1", "memory-policy-owner", "policies", _, _, _]
            )
            | ("GET", ["", "v1", "memory-policy-owner", "reviews", _])
            | ("GET", ["", "v1", "workflow-runs", _])
            | (
                "GET",
                [
                    "",
                    "v1",
                    "workflow-runs",
                    _,
                    "context-owner-association"
                        | "context-owner-source-status"
                        | "context-owner-effective-limits"
                        | "context"
                        | "provider-sessions"
                        | "context-owner-published-sources"
                        | "context-owner-drafts"
                        | "provider-session-effective-limits"
                        | "context-memory-effective-limits"
                        | "process-lifecycle"
                        | "provider-session-policy"
                ]
            )
            | (
                "GET",
                [
                    "",
                    "v1",
                    "workflow-runs",
                    _,
                    "artifacts"
                        | "context-owner-drafts"
                        | "context-owner-revisions"
                        | "context-owner-previews",
                    _
                ]
            )
            | (
                "GET",
                [
                    "",
                    "v1",
                    "workflow-runs",
                    _,
                    "executions",
                    _,
                    "context-binding"
                ]
            )
            | ("POST", ["", "v1", "workflow-targets", "preflight"])
            | ("POST", ["", "v1", "workflow-runs", _, "replay" | "export"])
            | (
                "POST",
                [
                    "",
                    "v1",
                    "workflow-runs",
                    _,
                    "context-control-receipts"
                        | "context-owner-mutation-receipts"
                        | "context-owner-draft-publication-receipts",
                    "lookup"
                ]
            )
    ) {
        RouteEffect::ReadOnly
    } else {
        RouteEffect::MayWrite
    }
}

/// Only the exact read route may spend the executor's reserved recovery slot.
pub(super) fn is_submission_recovery(request: &HttpRequest) -> bool {
    request.method == "GET"
        && request.query.is_empty()
        && request
            .path
            .strip_prefix("/v1/workflow-submissions/")
            .is_some_and(|id| validate_identifier("request_id", id).is_ok())
}
