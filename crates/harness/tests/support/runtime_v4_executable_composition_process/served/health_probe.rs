// SPDX-License-Identifier: MIT

//! Deciding whether the answer on the workflow address came from the service we spawned.
//!
//! Split out of `session` so that module stays inside the repository's preferred test-file size
//! budget. It is its own module because it is its own concern: the rest of `session` builds and
//! drives children, while this is the one predicate that says whether an address is ours.

use sts2_harness::management::{ClientResponse, MANAGEMENT_SCHEMA_VERSION};

/// The real service answers `/v1/health` with its own envelope and nothing else.
///
/// `http_routes.rs` serves this path ahead of authentication and ahead of every other route, and
/// `service_ops_lifecycle::health` is its only writer, so a 200 carrying `status == "ok"` and
/// this schema version is the service's identity rather than a property any other listener on
/// the address would happen to share.
///
/// The predicate is deliberately narrow. `ManagementClient::request_json` is `Ok` for *any*
/// completed exchange, so a readiness probe that asks only whether the call succeeded adopts
/// whatever answered — which is how sts2-harness#651 failed on a provider fixture's error
/// *identity* while the real cause was that the synthetic downstream had taken this port and the
/// harness child had died at its own `bind`.
pub(super) fn is_workflow_service_health(health: &ClientResponse) -> bool {
    if health.status != 200 {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&health.body)
        .ok()
        .is_some_and(|body| {
            body["status"] == "ok" && body["schema_version"] == MANAGEMENT_SCHEMA_VERSION
        })
}

/// Name what answered, so a squatted address is reported as a squatted address.
///
/// A timeout that only said "readiness deadline exceeded" is the report sts2-harness#651 could
/// not produce and the reader could not act on. This keeps the impostor's own status and body in
/// the message, which is the evidence needed to tell a port theft from a slow start.
pub(super) fn describe_unexpected_health(health: &ClientResponse) -> String {
    let body = String::from_utf8_lossy(&health.body);
    let body = body.trim();
    let body = if body.is_empty() {
        "<empty body>"
    } else {
        body
    };
    format!("HTTP {} {body}", health.status)
}
