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
}
