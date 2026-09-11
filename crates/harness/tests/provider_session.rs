// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

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

fn broker() -> ProviderSessionBroker {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sha256_hex("codex-app-server-fixture-v1");
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
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    broker
        .complete_candidate("owner-fixture", &operation.operation_id, "native-thread-1")
        .expect("complete candidate")
}

#[test]
fn feature_off_is_explicit_and_does_not_create_a_worker() {
    let test_scope = scope();
    let mut broker = ProviderSessionBroker::new(
        test_scope.clone(),
        ProviderSessionPolicy::disabled(test_scope),
        NativeCapabilities::fixture(),
        "owner-fixture",
    )
    .expect("broker");
    assert_eq!(
        broker.create_candidate(
            "owner-fixture",
            "create-1",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        ),
        Err(SessionError::Forbidden)
    );
    assert!(broker.bindings().next().is_none());

    let mut capabilities = NativeCapabilities::fixture();
    capabilities
        .enabled_methods
        .push("shell/execute".to_owned());
    let test_scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(test_scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sha256_hex("codex-app-server-fixture-v1");
    assert!(matches!(
        ProviderSessionBroker::new(test_scope, policy, capabilities, "owner-fixture",),
        Err(SessionError::InvalidCapabilities)
    ));
}

#[test]
fn fixture_capabilities_do_not_claim_native_encrypted_persistence() {
    let capabilities = NativeCapabilities::fixture();
    assert!(!capabilities.hardening.encrypted_state);
    assert!(capabilities.validate().is_ok());

    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::Enabled;
    policy.credential_realm_ref = "approved-realm".to_owned();
    policy.profile_sha256 = sha256_hex("codex-app-server-fixture-v1");
    let mut enabled_capabilities = capabilities;
    enabled_capabilities.strict_executable = true;
    assert!(matches!(
        ProviderSessionBroker::new(scope, policy, enabled_capabilities, "owner-fixture",),
        Err(SessionError::Unsupported)
    ));
}

#[test]
fn candidate_reconnect_and_history_remain_held() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let reconnect = broker
        .reconnect("owner-fixture", &binding.binding_id, "reconnect-1")
        .expect("reconnect");
    assert_eq!(
        broker
            .complete_reconnect(
                "owner-fixture",
                &reconnect.operation_id,
                &sha256_hex("continuity"),
                HistoryCoverage::ReportedPartial,
            )
            .expect("reconnect completion")
            .state,
        BindingState::Held
    );
    broker
        .refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-1",
            vec![HistoryItem {
                item_ref: "item-1".to_owned(),
                turn_ref: "turn-1".to_owned(),
                sequence: 1,
                kind: HistoryItemKind::UserInput,
                content_ref: Some("opaque-1".to_owned()),
                redacted: true,
            }],
            1,
            false,
        )
        .expect("history");
    let view = broker.history(&binding.binding_id, None, 8).expect("view");
    assert_eq!(view.coverage, HistoryCoverageView::Partial);
    assert_eq!(view.effective_context_coverage, "unknown");
    assert!(view.items[0].redacted);
}

#[test]
fn prepared_suffix_requires_explicit_resume_and_fences_unknown_turn() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let prepared = broker
        .prepare_turn(
            "owner-fixture",
            &binding.binding_id,
            "prepared-1",
            "preview-1",
            "revision-1",
            "selection-1",
            "boundary-1",
            br#"{"schema":"turn.v1","input":"exact"}"#.to_vec(),
            br#"{"type":"object"}"#.to_vec(),
            b"protected".to_vec(),
            Vec::new(),
            "2099-01-01T00:00:00Z",
        )
        .expect("prepared");
    assert_eq!(
        broker.admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-1"
        ),
        Err(SessionError::HeldRequired)
    );
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");
    let operation = broker
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-1",
        )
        .expect("admit turn");
    broker
        .mark_sent("owner-fixture", &operation.operation_id)
        .expect("sent");
    broker
        .acknowledge("owner-fixture", &operation.operation_id, "native-turn-1")
        .expect("ack");
    broker
        .complete_turn(
            "owner-fixture",
            &operation.operation_id,
            "native-turn-1",
            HistoryItem {
                item_ref: "decision-1".to_owned(),
                turn_ref: "native-turn-1".to_owned(),
                sequence: 2,
                kind: HistoryItemKind::ValidatedDecision,
                content_ref: None,
                redacted: true,
            },
        )
        .expect("complete");
    let second = broker
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-2",
        )
        .expect("second turn");
    let reconciliation = broker
        .mark_unknown("owner-fixture", &second.operation_id)
        .expect("unknown");
    assert_eq!(reconciliation.outcome, ReconciliationOutcome::Ambiguous);
    assert_eq!(reconciliation.new_generation_calls, 0);
    assert_eq!(
        broker.binding(&binding.binding_id).expect("binding").state,
        BindingState::Recovering
    );
}

#[test]
fn evaluation_fork_and_compaction_have_no_game_effects() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let fork = broker
        .plan_fork(
            "owner-fixture",
            &binding.binding_id,
            "fork-plan-1",
            "turn-0",
            0,
            ForkOperation::NativeFork,
            Vec::new(),
        )
        .expect("fork plan");
    assert!(!fork.game_dispatch_capability);
    let forked = broker
        .complete_fork("owner-fixture", &fork.fork_plan_id, "native-fork-1")
        .expect("fork completion");
    assert_eq!(forked.purpose, SessionPurpose::Evaluation);
    assert!(!forked.game_dispatch_capability);

    let job = broker
        .plan_compaction(
            "owner-fixture",
            &binding.binding_id,
            "compact-1",
            Some("budget-1".to_owned()),
            true,
        )
        .expect("compaction plan");
    assert_eq!(job.state, CompactionState::Planned);
    broker
        .send_compaction("owner-fixture", &job.job_id)
        .expect("send");
    let acknowledged = broker
        .acknowledge_compaction("owner-fixture", &job.job_id)
        .expect("ack");
    assert!(acknowledged.ack_received);
    let completed = broker
        .complete_compaction("owner-fixture", &job.job_id, "compact-evidence-1")
        .expect("complete");
    assert_eq!(completed.game_effects, 0);
    assert_eq!(completed.scheduler_after, "held");
}

#[test]
fn retirement_and_recovery_prevent_resurrection() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");
    let epoch = broker.recover("owner-fixture").expect("recover");
    assert_eq!(epoch, 2);
    assert_eq!(
        broker.binding(&binding.binding_id).expect("binding").state,
        BindingState::Recovering
    );
    let retirement = broker
        .retire(
            "owner-fixture",
            &binding.binding_id,
            "retire-1",
            vec!["source-1".to_owned()],
        )
        .expect("retire");
    assert!(retirement.admission_denied);
    let closed = broker
        .cleanup("owner-fixture", &retirement.retirement_id)
        .expect("cleanup");
    assert_eq!(closed.local_status, RetirementLocalStatus::Closed);
    assert_eq!(
        broker.binding(&binding.binding_id).expect("binding").state,
        BindingState::Closed
    );
}

#[test]
fn strict_frames_reject_duplicates_and_peer_runs_as_owned_stdio() {
    let duplicate = br#"{"jsonrpc":"2.0","id":1,"id":2,"result":{}}
"#;
    assert_eq!(parse_native_frame(duplicate), Err(SessionError::Protocol));
    let encoded = NativeFrame::response(1, serde_json::json!({"ok":true}))
        .encode_line()
        .expect("json-rpc frame");
    assert!(
        encoded
            .windows(br#""jsonrpc":"2.0""#.len())
            .any(|window| { window == br#""jsonrpc":"2.0""# })
    );
    assert!(matches!(
        parse_native_frame(&encoded),
        Ok(NativeResponse::Result { id: 1, .. })
    ));
    let unknown = br#"{"jsonrpc":"2.0","id":1,"result":{},"untrusted":true}
"#;
    assert_eq!(parse_native_frame(unknown), Err(SessionError::Protocol));
    let ambiguous = br#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":-1,"message":"both"}}
"#;
    assert_eq!(parse_native_frame(ambiguous), Err(SessionError::Protocol));
    let server = NativeFrame::server_request(4, "shell/execute")
        .encode_line()
        .expect("frame");
    assert!(matches!(
        parse_native_frame(&server),
        Ok(NativeResponse::ServerRequest { .. })
    ));

    let mut transport = OwnedNativeTransport::fixture_peer().expect("fixture peer");
    transport.start().expect("start");
    let initialized = transport.initialize().expect("initialize");
    assert_eq!(initialized["tools"], false);
    assert_eq!(
        transport.start_thread().expect("thread")["thread_id"],
        "native-thread-1"
    );
    assert!(transport.notification_count() <= 1);
    transport.close().expect("close");
}

fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    sts2_harness::sha256_hex(value)
}
