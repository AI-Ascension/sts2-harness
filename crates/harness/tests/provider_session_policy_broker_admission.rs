// SPDX-License-Identifier: MIT

//! Production-boundary conformance for saved provider-session policy admission.
//!
//! `ProviderSessionPolicy::admit_for_profile` must be the classification the runtime broker uses
//! when it admits a saved policy, not a library-only helper. A schema-valid policy the selected
//! adapter profile cannot execute is refused when the broker is constructed, before any turn is
//! prepared or any transport is touched.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use sts2_harness::provider_session::*;

fn scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

fn policy(turns: usize, ttl: u64) -> ProviderSessionPolicy {
    let mut policy = ProviderSessionPolicy::disabled(scope());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    policy.max_completed_turns = turns;
    policy.history_ttl_seconds = ttl;
    policy
}

fn admit(policy: ProviderSessionPolicy) -> Result<ProviderSessionBroker, SessionError> {
    ProviderSessionBroker::new(
        scope(),
        policy,
        NativeCapabilities::fixture(),
        "owner-token",
    )
}

#[test]
fn broker_admits_a_policy_within_the_executable_ceiling() {
    let broker =
        admit(policy(MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS)).expect("executable policy");
    assert_eq!(broker.policy().max_completed_turns, MAX_COMPLETED_TURNS);
    assert_eq!(broker.policy().history_ttl_seconds, MAX_HISTORY_TTL_SECONDS);
}

#[test]
fn broker_refuses_a_policy_one_over_the_executable_ceiling() {
    for refused in [
        policy(MAX_COMPLETED_TURNS + 1, 1),
        policy(1, MAX_HISTORY_TTL_SECONDS + 1),
    ] {
        let error = admit(refused).expect_err("not executable on the selected profile");
        assert_eq!(error, SessionError::Unsupported);
    }
}

/// Falsification probe for the production boundary: the inline ceiling comparison this replaced
/// accepted any within-ceiling policy on a profile that executes nothing, so removing the
/// `admit_for_profile` call makes this broker admit and then fail later as `InvalidCapabilities`
/// instead of refusing the unexecutable policy first.
#[test]
fn broker_refuses_a_saved_policy_the_selected_profile_disables() {
    let mut capabilities = NativeCapabilities::fixture();
    capabilities.enabled_methods.clear();
    let error = ProviderSessionBroker::new(scope(), policy(1, 1), capabilities, "owner-token")
        .expect_err("a disabled profile executes nothing, so the broker must refuse");
    assert_eq!(error, SessionError::Unsupported);
}
