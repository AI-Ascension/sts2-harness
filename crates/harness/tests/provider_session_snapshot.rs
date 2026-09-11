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
fn snapshot_is_metadata_only_and_idempotency_conflicts() {
    let mut broker = broker();
    let first = broker
        .create_candidate(
            "owner-fixture",
            "same-key",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    let same = broker
        .create_candidate(
            "owner-fixture",
            "same-key",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("same candidate");
    assert_eq!(first.operation_id, same.operation_id);
    assert_eq!(
        broker.create_candidate(
            "owner-fixture",
            "same-key",
            "branch-b",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z"
        ),
        Err(SessionError::Conflict)
    );
    let bytes = broker.snapshot_json().expect("snapshot");
    assert!(!String::from_utf8_lossy(&bytes).contains("provider_internal_context"));
    let mut duplicate = bytes.clone();
    assert_eq!(duplicate.pop(), Some(b'}'));
    duplicate.extend_from_slice(br#","schema":"duplicate"}"#);
    assert!(matches!(
        ProviderSessionBroker::from_snapshot_json(&duplicate, "replacement-owner"),
        Err(SessionError::Protocol)
    ));
}

#[test]
fn snapshot_restore_rehydrates_metadata_and_rotates_owner() {
    let mut broker = broker();
    let first = broker
        .create_candidate(
            "owner-fixture",
            "persisted-key",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    let binding = broker
        .complete_candidate("owner-fixture", &first.operation_id, "native-thread-1")
        .expect("complete candidate");
    broker
        .refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-1",
            vec![HistoryItem {
                item_ref: "item-1".to_owned(),
                turn_ref: "turn-1".to_owned(),
                sequence: 1,
                kind: HistoryItemKind::ValidatedDecision,
                content_ref: None,
                redacted: true,
            }],
            1,
            false,
        )
        .expect("history");

    let bytes = broker.snapshot_json().expect("snapshot");
    let mut restored =
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner").expect("restore");
    assert_eq!(
        restored
            .binding(&binding.binding_id)
            .expect("binding")
            .state,
        BindingState::Held
    );
    let restored_history = restored
        .history(&binding.binding_id, None, 8)
        .expect("history");
    assert_eq!(restored_history.items.len(), 1);
    assert!(restored_history.items[0].redacted);
    assert_eq!(
        restored
            .create_candidate(
                "replacement-owner",
                "persisted-key",
                "branch-a",
                SessionPurpose::Executable,
                "2099-01-01T00:00:00Z",
            )
            .expect("idempotent candidate")
            .operation_id,
        first.operation_id
    );
    assert_eq!(
        restored.create_candidate(
            "owner-fixture",
            "new-key",
            "branch-b",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        ),
        Err(SessionError::Unauthorized)
    );
    let next = restored
        .create_candidate(
            "replacement-owner",
            "new-key",
            "branch-b",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("new candidate");
    assert_ne!(next.operation_id, first.operation_id);
    assert_eq!(restored.owner_epoch(), 2);
}

#[test]
fn restore_applies_a_newer_retirement_tombstone_before_publication() {
    let mut broker = broker();
    let first = broker
        .create_candidate(
            "owner-fixture",
            "tombstone-candidate",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    let binding = broker
        .complete_candidate(
            "owner-fixture",
            &first.operation_id,
            "native-thread-tombstone",
        )
        .expect("complete candidate");
    let stale = broker.snapshot();
    let retirement = broker
        .retire(
            "owner-fixture",
            &binding.binding_id,
            "retirement-tombstone",
            vec!["source-revoked".to_owned()],
        )
        .expect("retire");
    let latest = broker.snapshot();
    let mut backup = stale;
    backup.revocation_epoch = latest.revocation_epoch;
    backup.retirements = vec![retirement];
    let bytes = serde_json::to_vec(&backup).expect("backup");
    let restored =
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner").expect("restore");
    assert_eq!(restored.owner_epoch(), 2);
    assert_eq!(
        restored
            .binding(&binding.binding_id)
            .expect("binding")
            .state,
        BindingState::Retired
    );
}

#[test]
fn restore_preserves_inflight_turn_as_unknown_and_held() {
    let mut broker = broker();
    let first = broker
        .create_candidate(
            "owner-fixture",
            "unknown-candidate",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    let binding = broker
        .complete_candidate(
            "owner-fixture",
            &first.operation_id,
            "native-thread-unknown",
        )
        .expect("complete candidate");
    let prepared = broker
        .prepare_turn(
            "owner-fixture",
            &binding.binding_id,
            "prepared-unknown",
            "preview-unknown",
            "revision-unknown",
            "selection-unknown",
            "boundary-unknown",
            b"suffix".to_vec(),
            br#"{"type":"object"}"#.to_vec(),
            Vec::new(),
            Vec::new(),
            "2099-01-01T00:00:00Z",
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
            "unknown-turn",
        )
        .expect("turn");
    broker
        .mark_sent("owner-fixture", &operation.operation_id)
        .expect("sent");
    let bytes = broker.snapshot_json().expect("snapshot");
    let restored =
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner").expect("restore");
    assert_eq!(
        restored
            .operation(&operation.operation_id)
            .expect("operation")
            .state,
        NativeOperationState::Unknown
    );
    assert_eq!(
        restored
            .binding(&binding.binding_id)
            .expect("binding")
            .state,
        BindingState::Recovering
    );
}

#[test]
fn checked_restore_rejects_profile_or_policy_drift() {
    let mut broker = broker();
    broker
        .create_candidate(
            "owner-fixture",
            "checked-restore-candidate",
            "branch-a",
            SessionPurpose::Executable,
            "2099-01-01T00:00:00Z",
        )
        .expect("candidate");
    let bytes = broker.snapshot_json().expect("snapshot");
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    let capabilities = NativeCapabilities::fixture();
    let restored = ProviderSessionBroker::from_snapshot_json_checked(
        &bytes,
        "replacement-owner",
        &scope,
        &policy,
        &capabilities,
    )
    .expect("checked restore");
    assert_eq!(restored.owner_epoch(), 2);

    let mut changed_capabilities = capabilities.clone();
    changed_capabilities.native_schema_sha256 = sts2_harness::sha256_hex("different-native-schema");
    assert!(matches!(
        ProviderSessionBroker::from_snapshot_json_checked(
            &bytes,
            "replacement-owner",
            &scope,
            &policy,
            &changed_capabilities,
        ),
        Err(SessionError::Unsupported)
    ));

    let mut changed_policy = policy;
    changed_policy.credential_realm_ref = "different-realm".to_owned();
    assert!(matches!(
        ProviderSessionBroker::from_snapshot_json_checked(
            &bytes,
            "replacement-owner",
            &scope,
            &changed_policy,
            &capabilities,
        ),
        Err(SessionError::Unsupported)
    ));
}
