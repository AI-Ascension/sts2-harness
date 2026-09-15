// SPDX-License-Identifier: MIT

//! Bounded, approval-gated migration records for saved provider-session policies.
//!
//! A saved policy that is portable-schema valid but above the selected profile's executable
//! ceiling must never be silently clamped: the harness records the violation, retains the exact
//! saved bytes, and requires explicit approval before a caller-supplied bounded target is adopted.
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

fn saved_bytes(turns: usize, ttl: u64) -> Vec<u8> {
    serde_json::to_vec(&policy(turns, ttl)).expect("saved bytes")
}

/// A caller-supplied bounded target: same identity, advanced version and epoch, executable values.
fn bounded_target(source: &ProviderSessionPolicy) -> ProviderSessionPolicy {
    let mut target = source.clone();
    target.version = source.version + 1;
    target.epoch = source.epoch + 1;
    target.max_completed_turns = MAX_COMPLETED_TURNS;
    target.history_ttl_seconds = MAX_HISTORY_TTL_SECONDS;
    target
}

#[test]
fn an_executable_policy_has_no_violations_and_raises_no_proposal() {
    let capabilities = NativeCapabilities::fixture();
    let executable = policy(MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS);
    assert!(
        executable
            .capability_limit_violations(&capabilities)
            .is_empty()
    );
    assert_eq!(
        SessionPolicyMigrationProposal::new_from_bytes(
            serde_json::to_vec(&executable).expect("bytes"),
            &capabilities,
            "proposal-1",
        ),
        Err(SessionPolicyMigrationError::InvalidProposal),
        "a proposal is only valid for a genuinely violated limit"
    );
}

#[test]
fn a_schema_valid_unsupported_policy_raises_a_proposal_retaining_exact_bytes() {
    let capabilities = NativeCapabilities::fixture();
    let bytes = saved_bytes(1_024, 604_800);
    let source: ProviderSessionPolicy = serde_json::from_slice(&bytes).expect("saved policy");
    source.validate_schema().expect("portable schema validity");

    let proposal = SessionPolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "p-1")
        .expect("proposal");
    assert_eq!(proposal.state, SessionPolicyMigrationState::Proposed);
    assert!(proposal.approval_ref.is_none());
    assert_eq!(proposal.original_policy_bytes(), bytes.as_slice());
    assert_eq!(
        proposal.source_policy_sha256,
        sts2_harness::sha256_hex(&bytes),
        "the digest must cover the exact saved bytes"
    );
    assert_eq!(proposal.violations.len(), 2);
    assert_eq!(proposal.violations[0].limit, "max_completed_turns");
    assert_eq!(proposal.violations[0].requested, 1_024);
    assert_eq!(proposal.violations[0].effective, MAX_COMPLETED_TURNS as u64);
    assert_eq!(proposal.violations[1].limit, "max_history_ttl_seconds");
    assert_eq!(proposal.violations[1].requested, 604_800);
    assert_eq!(proposal.violations[1].effective, MAX_HISTORY_TTL_SECONDS);
}

#[test]
fn a_portable_invalid_policy_cannot_be_proposed() {
    let capabilities = NativeCapabilities::fixture();
    // 1_025 exceeds the portable schema ceiling, so this is not a profile problem at all.
    let bytes = saved_bytes(1_025, 604_801);
    assert_eq!(
        SessionPolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "p-1"),
        Err(SessionPolicyMigrationError::InvalidProposal)
    );
}

#[test]
fn adoption_requires_explicit_approval_for_this_exact_proposal() {
    let capabilities = NativeCapabilities::fixture();
    let bytes = saved_bytes(129, 1);
    let source: ProviderSessionPolicy = serde_json::from_slice(&bytes).expect("saved");
    let target = bounded_target(&source);
    let mut proposal = SessionPolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "p-1")
        .expect("proposal");

    // Unapproved adoption is refused outright.
    assert_eq!(
        proposal.adopt(&target, &capabilities, "operator-1"),
        Err(SessionPolicyMigrationError::PermissionDenied)
    );
    // A malformed approval reference is refused.
    assert_eq!(
        proposal.approve("no spaces allowed"),
        Err(SessionPolicyMigrationError::PermissionDenied)
    );
    proposal.approve("operator-1").expect("approval");
    assert_eq!(proposal.state, SessionPolicyMigrationState::Approved);
    // An approval naming a different reference is refused.
    assert_eq!(
        proposal.adopt(&target, &capabilities, "someone-else"),
        Err(SessionPolicyMigrationError::PermissionDenied)
    );
    // The approved reference adopts, and the exact saved bytes are still retained.
    let adopted = proposal
        .adopt(&target, &capabilities, "operator-1")
        .expect("adoption");
    assert_eq!(adopted, target);
    assert_eq!(proposal.state, SessionPolicyMigrationState::Adopted);
    assert!(proposal.adopted_policy_sha256.is_some());
    assert_eq!(proposal.original_policy_bytes(), bytes.as_slice());
}

#[test]
fn a_still_violating_target_is_refused_instead_of_being_clamped() {
    let capabilities = NativeCapabilities::fixture();
    let bytes = saved_bytes(1_024, 1);
    let source: ProviderSessionPolicy = serde_json::from_slice(&bytes).expect("saved");
    let mut proposal = SessionPolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "p-1")
        .expect("proposal");
    proposal.approve("operator-1").expect("approval");

    // A "target" that is still over the executable ceiling must not be silently clamped.
    let mut over = bounded_target(&source);
    over.max_completed_turns = MAX_COMPLETED_TURNS + 1;
    let error = proposal
        .adopt(&over, &capabilities, "operator-1")
        .expect_err("a violating target is refused");
    assert_eq!(
        error,
        SessionPolicyMigrationError::CapabilityLimitExceeded {
            limit: "max_completed_turns".to_owned(),
            requested: (MAX_COMPLETED_TURNS + 1) as u64,
            effective: MAX_COMPLETED_TURNS as u64,
        }
    );
    assert_eq!(error.code(), "effective_limit_exceeded");
    assert_eq!(proposal.state, SessionPolicyMigrationState::Approved);
    assert!(proposal.adopted_policy_sha256.is_none());
}

#[test]
fn a_target_for_another_capability_descriptor_is_refused() {
    let capabilities = NativeCapabilities::fixture();
    let bytes = saved_bytes(129, 1);
    let source: ProviderSessionPolicy = serde_json::from_slice(&bytes).expect("saved");
    let mut proposal = SessionPolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "p-1")
        .expect("proposal");
    proposal.approve("operator-1").expect("approval");

    let mut other = NativeCapabilities::fixture();
    other.binding.descriptor_sha256 = sts2_harness::sha256_hex("another-profile");
    assert_eq!(
        proposal.adopt(&bounded_target(&source), &other, "operator-1"),
        Err(SessionPolicyMigrationError::InvalidCapabilities)
    );
}

#[test]
fn tampered_source_bytes_fail_validation() {
    let capabilities = NativeCapabilities::fixture();
    let bytes = saved_bytes(129, 1);
    let mut proposal = SessionPolicyMigrationProposal::new_from_bytes(&bytes, &capabilities, "p-1")
        .expect("proposal");
    proposal.original_policy_bytes = saved_bytes(130, 1);
    assert_eq!(
        proposal.validate(),
        Err(SessionPolicyMigrationError::InvalidProposal),
        "the retained digest must match the retained bytes"
    );
}
