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

#[test]
fn lower_profile_operation_limit_is_enforced() {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    let mut capabilities = NativeCapabilities::fixture();
    capabilities.effective_limits.max_operations = 1;
    capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
    let mut broker =
        ProviderSessionBroker::new(scope, policy, capabilities, "owner-fixture").expect("broker");
    broker
        .create_candidate(
            "owner-fixture",
            "limited-create-1",
            "branch-a",
            SessionPurpose::Executable,
            support::expiry(),
        )
        .expect("first operation");
    assert_eq!(
        broker.create_candidate(
            "owner-fixture",
            "limited-create-2",
            "branch-a",
            SessionPurpose::Executable,
            support::expiry(),
        ),
        Err(SessionError::Capacity)
    );
}
