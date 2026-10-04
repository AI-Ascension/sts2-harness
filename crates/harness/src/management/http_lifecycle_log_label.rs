// SPDX-License-Identifier: MIT

//! Field labelling and redaction for the management request-lifecycle log.
//!
//! Split out of `http_lifecycle_log` because route identity and redaction are
//! separate from when request markers are emitted.
//!
//! Served paths are matched by method and full route shape. A match emits a fixed
//! route template with dynamic components replaced by placeholders. An unmatched
//! path receives one fixed label, so a caller identifier cannot become visible just
//! because it happens to equal a safe-looking static word. Query values are dropped,
//! and every output is bounded and single-line.

use super::MAX_ROUTE_BYTES;

/// Reduce an HTTP method to a bounded, loggable token.
///
/// The server admits only `GET`, `POST` and `PUT`; anything else is reported as
/// `other` rather than echoed.
pub(super) fn method_label(method: &str) -> String {
    if matches!(method, "GET" | "POST" | "PUT") {
        format!("method={method}")
    } else {
        String::from("method=other")
    }
}

/// Method-aware route templates served by the management HTTP adapter.
///
/// Placeholders are emitted as fixed labels, never as request values. The list is
/// intentionally exact: a new handler is redacted as unmatched until its route
/// template is reviewed and added here.
const ROUTE_TEMPLATES: &[(&str, &str)] = &[
    ("GET", "/v1/health"),
    ("POST", "/v1/workflow-definitions/validate"),
    ("POST", "/v1/workflow-definitions/inspect"),
    ("POST", "/v1/workflow-definitions/diff"),
    ("POST", "/v1/workflow-runs"),
    ("GET", "/v1/workflow-targets"),
    ("POST", "/v1/workflow-targets/preflight"),
    ("GET", "/v1/context-bindings"),
    ("POST", "/v1/context-bindings/bind"),
    ("GET", "/v1/inference-profiles"),
    ("POST", "/v1/inference-profiles/{profile_id}/revisions"),
    ("GET", "/v1/capabilities"),
    ("GET", "/v1/studio/definitions"),
    ("POST", "/v1/studio/drafts"),
    ("GET", "/v1/studio/drafts/{draft_id}"),
    ("PUT", "/v1/studio/drafts/{draft_id}"),
    ("POST", "/v1/studio/drafts/{draft_id}/publish"),
    ("POST", "/v1/studio/authoring-inference"),
    (
        "GET",
        "/v1/studio/authoring-inference/operations/{draft_id}/{client_mutation_id}",
    ),
    ("GET", "/v1/memory-policy-owner"),
    (
        "GET",
        "/v1/memory-policy-owner/policies/{policy_id}/{version}/{raw_sha256}",
    ),
    ("GET", "/v1/memory-policy-owner/reviews/{review_id}"),
    ("POST", "/v1/memory-policy-owner/proposals/{review_id}"),
    (
        "POST",
        "/v1/memory-policy-owner/proposals/{review_id}/approve",
    ),
    (
        "POST",
        "/v1/memory-policy-owner/proposals/{review_id}/adopt",
    ),
    ("GET", "/v1/workflow-runs/{run_id}"),
    (
        "GET",
        "/v1/workflow-runs/{run_id}/context-owner-association",
    ),
    (
        "GET",
        "/v1/workflow-runs/{run_id}/context-owner-source-status",
    ),
    (
        "GET",
        "/v1/workflow-runs/{run_id}/context-owner-effective-limits",
    ),
    ("GET", "/v1/workflow-runs/{run_id}/context"),
    (
        "PUT",
        "/v1/workflow-runs/{run_id}/context-sources/{source_id}",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/context-sources/{source_id}/adopt",
    ),
    (
        "GET",
        "/v1/workflow-runs/{run_id}/executions/{node_execution_id}/context-binding",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/context-control-receipts/lookup",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/context-control-commands",
    ),
    ("GET", "/v1/workflow-runs/{run_id}/provider-sessions"),
    ("GET", "/v1/workflow-runs/{run_id}/events"),
    ("POST", "/v1/workflow-runs/{run_id}/commands"),
    ("POST", "/v1/workflow-runs/{run_id}/replay"),
    ("POST", "/v1/workflow-runs/{run_id}/export"),
    ("GET", "/v1/workflow-runs/{run_id}/artifacts/{artifact_id}"),
    ("GET", "/v1/workflow-runs/{run_id}/provider-session-policy"),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/provider-session-policy/import",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/provider-session-policy/proposals/{proposal_id}",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/provider-session-policy/proposals/{proposal_id}/approve",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/provider-session-policy/proposals/{proposal_id}/adopt",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/provider-session-policy/adoptions",
    ),
    (
        "GET",
        "/v1/workflow-runs/{run_id}/provider-session-effective-limits",
    ),
    (
        "GET",
        "/v1/workflow-runs/{run_id}/context-memory-effective-limits",
    ),
    ("GET", "/v1/workflow-runs/{run_id}/process-lifecycle"),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/process-lifecycle/operations",
    ),
    (
        "POST",
        "/v1/workflow-runs/{run_id}/process-lifecycle/operations/{operation_id}/reconcile",
    ),
];

/// Reduce a request target to a bounded, loggable route template.
///
/// Queries are caller-controlled and discarded. A full method-and-shape match emits
/// the fixed served template; every other path collapses to `route=unmatched`.
pub(super) fn route_label(method: &str, target: &str) -> String {
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    let template = ROUTE_TEMPLATES
        .iter()
        .find_map(|(route_method, template)| {
            (*route_method == method && path_matches_template(template, path)).then_some(*template)
        })
        .unwrap_or("unmatched");
    let label = format!("route={template}");
    if label.len() <= MAX_ROUTE_BYTES {
        label
    } else {
        String::from("route=unmatched")
    }
}

fn path_matches_template(template: &str, path: &str) -> bool {
    let mut path_segments = path.split('/');
    for template_segment in template.split('/') {
        let Some(path_segment) = path_segments.next() else {
            return false;
        };
        let is_parameter = template_segment.starts_with('{') && template_segment.ends_with('}');
        if is_parameter {
            if path_segment.is_empty() {
                return false;
            }
        } else if template_segment != path_segment {
            return false;
        }
    }
    path_segments.next().is_none()
}

/// Reduce a typed error code to a bounded, loggable token.
///
/// Codes are harness-owned constants, so this is a belt-and-braces bound rather than
/// trust: an unexpected value is still clamped and stripped.
pub(super) fn code_label(code: &str) -> String {
    bounded_label("code=", code)
}

/// The abandonment `reason=` field.
///
/// Deliberately the same bounded token as `code=`, not the error's message: an
/// `io::Error` string can carry caller bytes, and `reason=` must never become a
/// second channel for them. Two identical fields are cheaper than a leak.
pub(super) fn reason_label(code: &str) -> String {
    bounded_label("reason=", code)
}

/// Clamp `text` into a single-line label with a strict byte cap.
fn bounded_label(prefix: &str, text: &str) -> String {
    const TRUNCATION: &str = "..truncated";
    let mut label = String::from(prefix);
    let max_without_suffix = MAX_ROUTE_BYTES.saturating_sub(TRUNCATION.len());
    for character in text.chars() {
        let safe_character = if character.is_ascii_graphic() {
            character
        } else {
            // Replaced, not skipped: skipping would silently join two segments into
            // one misleading token. A space cannot forge a new field boundary.
            ' '
        };
        if label.len() + safe_character.len_utf8() > max_without_suffix {
            label.push_str(TRUNCATION);
            break;
        }
        label.push(safe_character);
    }
    label
}
