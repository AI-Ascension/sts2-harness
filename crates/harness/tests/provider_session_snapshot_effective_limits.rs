// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::provider_session::*;

#[path = "support/provider_session.rs"]
mod support;

fn scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

fn broker() -> ProviderSessionBroker {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    ProviderSessionBroker::new(
        scope,
        policy,
        NativeCapabilities::fixture(),
        "owner-fixture",
    )
    .expect("broker")
}

#[test]
fn restore_rejects_operations_and_events_above_effective_limits() {
    let mut broker = broker();
    for idempotency_key in ["restore-operation-a", "restore-operation-b"] {
        broker
            .create_candidate(
                "owner-fixture",
                idempotency_key,
                "branch-a",
                SessionPurpose::Executable,
                support::expiry(),
            )
            .expect("candidate");
    }

    let snapshot = broker.snapshot();
    assert!(snapshot.operations.len() >= 2);
    assert!(snapshot.events.len() >= 2);

    let mut operations_limited = snapshot.clone();
    operations_limited
        .capabilities
        .effective_limits
        .max_operations = 1;
    operations_limited.capabilities.binding.descriptor_sha256 =
        operations_limited.capabilities.descriptor_digest();
    let bytes = serde_json::to_vec(&operations_limited).expect("operations snapshot bytes");
    assert!(matches!(
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner"),
        Err(SessionError::Capacity)
    ));

    let mut events_limited = snapshot;
    events_limited.capabilities.effective_limits.max_events = 1;
    events_limited.capabilities.binding.descriptor_sha256 =
        events_limited.capabilities.descriptor_digest();
    let bytes = serde_json::to_vec(&events_limited).expect("events snapshot bytes");
    assert!(matches!(
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner"),
        Err(SessionError::Capacity)
    ));
}
