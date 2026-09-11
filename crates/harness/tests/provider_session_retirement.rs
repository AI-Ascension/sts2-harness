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

fn held_binding(broker: &mut ProviderSessionBroker) -> SessionBinding {
    let operation = broker
        .create_candidate(
            "owner-fixture",
            "create-retirement",
            "branch-retirement",
            SessionPurpose::Executable,
            support::expiry(),
        )
        .expect("candidate");
    broker
        .complete_candidate(
            "owner-fixture",
            &operation.operation_id,
            "native-thread-retirement",
        )
        .expect("complete candidate")
}

#[test]
fn late_maintenance_completions_cannot_resurrect_retired_bindings() {
    let mut candidate_broker = broker();
    let candidate = candidate_broker
        .create_candidate(
            "owner-fixture",
            "late-candidate",
            "branch-late-candidate",
            SessionPurpose::Evaluation,
            support::expiry(),
        )
        .expect("candidate");
    let candidate_binding = candidate_broker
        .operation(&candidate.operation_id)
        .expect("candidate operation")
        .binding_id
        .clone();
    candidate_broker
        .retire(
            "owner-fixture",
            &candidate_binding,
            "retire-late-candidate",
            Vec::new(),
        )
        .expect("retire candidate");
    assert_eq!(
        candidate_broker.complete_candidate(
            "owner-fixture",
            &candidate.operation_id,
            "late-native-candidate",
        ),
        Err(SessionError::Retired)
    );
    assert_eq!(
        candidate_broker
            .binding(&candidate_binding)
            .expect("candidate binding")
            .state,
        BindingState::Retired
    );

    let mut reconnect_broker = broker();
    let reconnect_binding = held_binding(&mut reconnect_broker);
    let reconnect = reconnect_broker
        .reconnect(
            "owner-fixture",
            &reconnect_binding.binding_id,
            "late-reconnect",
        )
        .expect("reconnect");
    reconnect_broker
        .retire(
            "owner-fixture",
            &reconnect_binding.binding_id,
            "retire-late-reconnect",
            Vec::new(),
        )
        .expect("retire reconnect");
    assert_eq!(
        reconnect_broker.complete_reconnect(
            "owner-fixture",
            &reconnect.operation_id,
            &sts2_harness::sha256_hex("late-continuity"),
            HistoryCoverage::ReportedPartial,
        ),
        Err(SessionError::Retired)
    );
    assert_eq!(
        reconnect_broker
            .binding(&reconnect_binding.binding_id)
            .expect("reconnect binding")
            .state,
        BindingState::Retired
    );

    let mut compaction_broker = broker();
    let compaction_binding = held_binding(&mut compaction_broker);
    let job = compaction_broker
        .plan_compaction(
            "owner-fixture",
            &compaction_binding.binding_id,
            "late-compaction",
            Some("late-budget".to_owned()),
            true,
        )
        .expect("compaction");
    compaction_broker
        .send_compaction("owner-fixture", &job.job_id)
        .expect("send compaction");
    compaction_broker
        .acknowledge_compaction("owner-fixture", &job.job_id)
        .expect("ack compaction");
    compaction_broker
        .retire(
            "owner-fixture",
            &compaction_binding.binding_id,
            "retire-late-compaction",
            Vec::new(),
        )
        .expect("retire compaction");
    assert_eq!(
        compaction_broker.complete_compaction(
            "owner-fixture",
            &job.job_id,
            "late-compaction-evidence",
        ),
        Err(SessionError::Retired)
    );
    assert_eq!(
        compaction_broker
            .binding(&compaction_binding.binding_id)
            .expect("compaction binding")
            .state,
        BindingState::Retired
    );
}

#[test]
fn owner_rotation_fences_pending_maintenance() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let fork = broker
        .plan_fork(
            "owner-fixture",
            &binding.binding_id,
            "rotation-fork",
            "turn-0",
            0,
            ForkOperation::NativeFork,
            Vec::new(),
        )
        .expect("fork");
    let job = broker
        .plan_compaction(
            "owner-fixture",
            &binding.binding_id,
            "rotation-compaction",
            Some("rotation-budget".to_owned()),
            true,
        )
        .expect("compaction");

    assert_eq!(
        broker.replace_owner("owner-fixture", "replacement-owner"),
        Ok(2)
    );
    assert_eq!(
        broker
            .binding(&fork.target_binding_id)
            .expect("fork target")
            .state,
        BindingState::Quarantined
    );
    assert_eq!(
        broker.complete_fork(
            "replacement-owner",
            &fork.fork_plan_id,
            "rotation-native-fork",
        ),
        Err(SessionError::Retired)
    );
    assert_eq!(
        broker.complete_compaction("replacement-owner", &job.job_id, "rotation-evidence",),
        Err(SessionError::Conflict)
    );
}
