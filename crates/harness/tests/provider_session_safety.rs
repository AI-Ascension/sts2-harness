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
        scope.clone(),
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
            "create-1",
            "branch-a",
            SessionPurpose::Executable,
            support::expiry(),
        )
        .expect("candidate");
    broker
        .complete_candidate("owner-fixture", &operation.operation_id, "native-thread-1")
        .expect("complete candidate")
}

#[test]
fn dependency_revocation_reaches_fork_and_interrupt_fences_late_result() {
    let mut session = broker();
    let source = held_binding(&mut session);
    session
        .set_dependencies(
            "owner-fixture",
            &source.binding_id,
            vec!["source-1".to_owned()],
        )
        .expect("dependencies");
    let fork = session
        .plan_fork(
            "owner-fixture",
            &source.binding_id,
            "fork-plan-revocation",
            "turn-0",
            0,
            ForkOperation::NativeFork,
            Vec::new(),
        )
        .expect("fork");
    assert_eq!(fork.dependency_ids, vec!["source-1"]);
    let forked = session
        .complete_fork(
            "owner-fixture",
            &fork.fork_plan_id,
            "native-fork-revocation",
        )
        .expect("fork completion");
    assert_eq!(forked.dependency_ids, vec!["source-1"]);
    let retired = session
        .revoke_sources("owner-fixture", vec!["source-1".to_owned()])
        .expect("revocation");
    assert_eq!(retired.len(), 2);
    assert_eq!(
        session.binding(&source.binding_id).expect("source").state,
        BindingState::Retired
    );
    assert_eq!(
        session.binding(&forked.binding_id).expect("fork").state,
        BindingState::Retired
    );

    let mut interrupting = broker();
    let binding = held_binding(&mut interrupting);
    let prepared = interrupting
        .prepare_turn(
            "owner-fixture",
            &binding.binding_id,
            "prepared-interrupt",
            "preview-interrupt",
            "revision-interrupt",
            "selection-interrupt",
            "boundary-interrupt",
            b"suffix".to_vec(),
            br#"{"type":"object"}"#.to_vec(),
            Vec::new(),
            Vec::new(),
            support::expiry(),
        )
        .expect("prepared");
    interrupting
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");
    let operation = interrupting
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "interrupt-key",
        )
        .expect("turn");
    interrupting
        .mark_sent("owner-fixture", &operation.operation_id)
        .expect("sent");
    let interrupted = interrupting
        .interrupt("owner-fixture", &operation.operation_id)
        .expect("interrupt");
    assert_eq!(interrupted.state, NativeOperationState::Unknown);
    assert_eq!(
        interrupting
            .binding(&binding.binding_id)
            .expect("binding")
            .state,
        BindingState::Recovering
    );
    assert_eq!(
        interrupting.complete_turn(
            "owner-fixture",
            &operation.operation_id,
            "late-turn",
            HistoryItem {
                item_ref: "late-item".to_owned(),
                turn_ref: "late-turn".to_owned(),
                sequence: 1,
                kind: HistoryItemKind::ValidatedDecision,
                content_ref: None,
                redacted: true,
            },
        ),
        Err(SessionError::Conflict)
    );
}

#[test]
fn transport_allowlist_and_json_depth_fail_closed() {
    let mut transport = OwnedNativeTransport::fixture_peer().expect("fixture peer");
    transport.start().expect("start");
    transport.initialize().expect("initialize");
    let turn = transport.start_turn().expect("turn");
    assert_eq!(turn["turn_ref"], "turn-1");
    transport.close().expect("close");

    let nested = format!(
        "{}0{}\n",
        "[".repeat(MAX_JSON_DEPTH + 1),
        "]".repeat(MAX_JSON_DEPTH + 1)
    );
    assert_eq!(
        parse_native_frame(nested.as_bytes()),
        Err(SessionError::Capacity)
    );
}

#[test]
fn conflicting_terminal_evidence_quarantines_the_attempt() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let prepared = broker
        .prepare_turn(
            "owner-fixture",
            &binding.binding_id,
            "prepared-terminal",
            "preview-terminal",
            "revision-terminal",
            "selection-terminal",
            "boundary-terminal",
            b"suffix".to_vec(),
            br#"{"type":"object"}"#.to_vec(),
            Vec::new(),
            Vec::new(),
            support::expiry(),
        )
        .expect("prepared");
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");
    let operation = broker
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "terminal-key",
        )
        .expect("turn");
    broker
        .mark_sent("owner-fixture", &operation.operation_id)
        .expect("sent");
    broker
        .acknowledge("owner-fixture", &operation.operation_id, "native-turn-a")
        .expect("ack");
    let item = HistoryItem {
        item_ref: "terminal-item".to_owned(),
        turn_ref: "native-turn-a".to_owned(),
        sequence: 1,
        kind: HistoryItemKind::ValidatedDecision,
        content_ref: None,
        redacted: true,
    };
    broker
        .complete_turn(
            "owner-fixture",
            &operation.operation_id,
            "native-turn-a",
            item.clone(),
        )
        .expect("complete");
    assert_eq!(
        broker
            .complete_turn(
                "owner-fixture",
                &operation.operation_id,
                "native-turn-a",
                item.clone(),
            )
            .expect("idempotent completion")
            .state,
        NativeOperationState::Completed
    );
    assert_eq!(
        broker.complete_turn(
            "owner-fixture",
            &operation.operation_id,
            "native-turn-b",
            item,
        ),
        Err(SessionError::Conflict)
    );
    assert_eq!(
        broker.binding(&binding.binding_id).expect("binding").state,
        BindingState::Quarantined
    );
}

#[test]
fn finite_turn_candidate_and_maintenance_quotas_cannot_be_bypassed() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    for index in 0..3 {
        let operation = broker
            .create_candidate(
                "owner-fixture",
                &format!("candidate-{index}"),
                &format!("branch-{index}"),
                SessionPurpose::Evaluation,
                support::expiry(),
            )
            .expect("candidate quota setup");
        assert_eq!(operation.state, NativeOperationState::IntentPersisted);
    }
    let fork = broker
        .plan_fork(
            "owner-fixture",
            &binding.binding_id,
            "fork-quota-1",
            "turn-0",
            0,
            ForkOperation::NativeFork,
            Vec::new(),
        )
        .expect("fourth candidate through fork");
    assert_eq!(
        broker.plan_fork(
            "owner-fixture",
            &binding.binding_id,
            "fork-quota-2",
            "turn-0",
            0,
            ForkOperation::NativeFork,
            Vec::new(),
        ),
        Err(SessionError::Capacity)
    );
    broker
        .complete_fork("owner-fixture", &fork.fork_plan_id, "native-fork-quota")
        .expect("complete fork");
    let forked_binding = fork.target_binding_id.clone();
    let job = broker
        .plan_compaction(
            "owner-fixture",
            &binding.binding_id,
            "compact-quota-1",
            Some("budget-quota-1".to_owned()),
            true,
        )
        .expect("maintenance quota setup");
    assert_eq!(job.state, CompactionState::Planned);
    assert_eq!(
        broker.plan_compaction(
            "owner-fixture",
            &forked_binding,
            "compact-quota-2",
            Some("budget-quota-2".to_owned()),
            true,
        ),
        Err(SessionError::Capacity)
    );
}

#[test]
fn history_refresh_enforces_policy_turn_quota() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let items = vec![
        HistoryItem {
            item_ref: "decision-a".to_owned(),
            turn_ref: "turn-a".to_owned(),
            sequence: 1,
            kind: HistoryItemKind::ValidatedDecision,
            content_ref: None,
            redacted: true,
        },
        HistoryItem {
            item_ref: "decision-b".to_owned(),
            turn_ref: "turn-b".to_owned(),
            sequence: 2,
            kind: HistoryItemKind::ValidatedDecision,
            content_ref: None,
            redacted: true,
        },
    ];
    assert_eq!(
        broker.refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-over-turn-limit",
            items,
            2,
            false,
        ),
        Err(SessionError::Capacity)
    );
}

#[test]
fn policy_rejects_retention_limits_above_declared_bounds() {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.max_completed_turns = MAX_COMPLETED_TURNS + 1;
    assert_eq!(policy.validate(), Err(SessionError::InvalidPolicy));
    policy.max_completed_turns = MAX_COMPLETED_TURNS;
    policy.history_ttl_seconds = MAX_HISTORY_TTL_SECONDS + 1;
    assert_eq!(policy.validate(), Err(SessionError::InvalidPolicy));
}
