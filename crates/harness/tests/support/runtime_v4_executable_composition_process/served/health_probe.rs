// SPDX-License-Identifier: MIT

//! Deciding whether the answer on the workflow address came from the service we spawned.
//!
//! Split out of `session` so that module stays inside the repository's preferred test-file size
//! budget. It is its own module because it is its own concern: the rest of `session` builds and
//! drives children, while this is the one predicate that says whether an address is ours.

use sts2_harness::management::{ClientResponse, MANAGEMENT_SCHEMA_VERSION};

// #651's regression scenarios for the predicate below. They hang off this module rather than
// off `session` so the probe and its proof of behaviour stay together, and so `session` stays
// inside the repository's preferred test-file size budget.
#[path = "health_probe_tests.rs"]
mod health_probe_tests;
pub(crate) use health_probe_tests::{
    the_workflow_health_predicate_accepts_only_the_services_own_envelope,
    the_workflow_readiness_probe_rejects_an_impostor_that_answers,
};

/// Wait for the spawned workflow service, accepting only its own health answer.
///
/// The loop itself lives beside the predicate rather than in `session`, because the two are one
/// decision: this is the only caller of [`is_workflow_service_health`], and keeping them together
/// is also what keeps `session` inside the repository's preferred test-file size budget.
pub(crate) fn wait_for_workflow_service(
    service: &mut std::process::Child,
    address: std::net::SocketAddr,
) -> Result<sts2_harness::management::ManagementClient, Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    let client = sts2_harness::management::ManagementClient::new(address, "served-workflow-token")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    // The last answer that was not the real service. Retained so a timeout can name the squatter
    // rather than reporting a bare deadline: an address served by something else must be
    // reported as that, never silently retried until the deadline. Declared before the loop
    // because the deadline can be reached on the first pass, before any arm has assigned.
    let mut impostor: Option<String>;
    loop {
        if let Some(status) = service.try_wait()? {
            return Err(format!("served workflow exited: {status}").into());
        }
        match client.request_json("GET", "/v1/health", None) {
            Ok(health) if is_workflow_service_health(&health) => return Ok(client),
            // `request_json` is `Ok` for *any* completed exchange, so a foreign listener that
            // answers at all used to satisfy this probe — the same squatter-satisfiable
            // readiness that #673 documented for `ready()`. On sts2-harness#651 the synthetic
            // downstream answered here and the scenario failed much later, on the provider
            // fixture's error *identity*, which named neither the theft nor the impostor.
            Ok(health) => {
                impostor = Some(describe_unexpected_health(&health));
            }
            Err(_) => impostor = None,
        }
        if Instant::now() >= deadline {
            return Err(match impostor {
                Some(description) => format!(
                    "served workflow readiness deadline exceeded, and the address was answering \
                     for something else: {description}"
                )
                .into(),
                None => "served workflow readiness deadline exceeded".into(),
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

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
