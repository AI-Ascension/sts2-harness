// SPDX-License-Identifier: MIT

//! Boundary conformance for saved provider-session policy admission.
//!
//! The portable schema ceiling and the executable ceiling are deliberately different values
//! (1,024/604,800 versus 128/86,400). A policy that is schema-valid but above the executable
//! ceiling must be refused with a precise capability reason, never clamped and never reported as
//! a generic invalid-policy failure.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use sts2_harness::effective_limits::UnavailableReason;
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

#[test]
fn lower_bound_and_exact_executable_boundaries_are_accepted() {
    let capabilities = NativeCapabilities::fixture();
    for (turns, ttl) in [(1, 1), (MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS)] {
        policy(turns, ttl)
            .admit_for_profile(&capabilities)
            .unwrap_or_else(|error| panic!("({turns}, {ttl}) should be executable: {error:?}"));
    }
}

#[test]
fn one_over_the_executable_ceiling_is_a_precise_profile_refusal() {
    let capabilities = NativeCapabilities::fixture();
    let turns = policy(MAX_COMPLETED_TURNS + 1, 1);
    let error = turns
        .admit_for_profile(&capabilities)
        .expect_err("one over the turn ceiling is not executable");
    assert_eq!(
        error,
        PolicyAdmissionError::Profile(UnavailableReason::EffectiveLimitExceeded)
    );
    assert_eq!(error.code(), "effective_limit_exceeded");

    let ttl = policy(1, MAX_HISTORY_TTL_SECONDS + 1);
    let error = ttl
        .admit_for_profile(&capabilities)
        .expect_err("one over the ttl ceiling is not executable");
    assert_eq!(
        error,
        PolicyAdmissionError::Profile(UnavailableReason::EffectiveLimitExceeded)
    );
}

#[test]
fn schema_valid_but_unsupported_yields_a_capability_error_not_a_schema_error() {
    let capabilities = NativeCapabilities::fixture();
    // Exactly the portable schema maxima: valid against the contract, far above what the
    // selected profile can execute.
    for (turns, ttl) in [(1_024, 1), (1, 604_800)] {
        let saved = policy(turns, ttl);
        saved.validate_schema().expect("portable schema validity");
        let error = saved
            .admit_for_profile(&capabilities)
            .expect_err("schema-valid but not executable");
        assert_eq!(
            error,
            PolicyAdmissionError::Profile(UnavailableReason::EffectiveLimitExceeded),
            "({turns}, {ttl}) must be classified as a profile limit, not a schema failure"
        );
        assert!(
            matches!(error, PolicyAdmissionError::Profile(_)),
            "a generic invalid-policy failure would hide the profile boundary"
        );
    }
}

#[test]
fn beyond_the_portable_schema_is_a_schema_failure() {
    let capabilities = NativeCapabilities::fixture();
    for (turns, ttl) in [(1_025, 1), (1, 604_801)] {
        let error = policy(turns, ttl)
            .admit_for_profile(&capabilities)
            .expect_err("beyond the portable schema");
        assert_eq!(
            error,
            PolicyAdmissionError::Schema(SessionError::InvalidPolicy),
            "({turns}, {ttl}) is outside the portable contract"
        );
        assert_eq!(error.code(), "provider_session_policy_schema_invalid");
    }
}

#[test]
fn admission_never_clamps_or_rewrites_the_saved_policy() {
    let capabilities = NativeCapabilities::fixture();
    let saved = policy(MAX_COMPLETED_TURNS + 1, MAX_HISTORY_TTL_SECONDS + 1);
    assert!(saved.admit_for_profile(&capabilities).is_err());
    // The refused policy keeps its saved values: nothing was silently clamped to fit.
    assert_eq!(saved.max_completed_turns, MAX_COMPLETED_TURNS + 1);
    assert_eq!(saved.history_ttl_seconds, MAX_HISTORY_TTL_SECONDS + 1);
    assert_eq!(saved.max_completed_turns, 129);
    assert_eq!(saved.history_ttl_seconds, 86_401);
}

#[test]
fn a_disabled_profile_refuses_with_the_disabled_reason() {
    let mut capabilities = NativeCapabilities::fixture();
    capabilities.enabled_methods.clear();
    let error = policy(1, 1)
        .admit_for_profile(&capabilities)
        .expect_err("a disabled surface executes nothing");
    assert_eq!(
        error,
        PolicyAdmissionError::Profile(UnavailableReason::Disabled)
    );
    assert_eq!(error.code(), "disabled");
}
