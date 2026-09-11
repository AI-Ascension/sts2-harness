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
            "create-cursor",
            "branch-cursor",
            SessionPurpose::Executable,
            support::expiry(),
        )
        .expect("candidate");
    broker
        .complete_candidate(
            "owner-fixture",
            &operation.operation_id,
            "native-thread-cursor",
        )
        .expect("complete candidate")
}

#[test]
fn history_cursor_binds_scope_binding_and_history_epoch() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    broker
        .refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-cursor",
            vec![
                HistoryItem {
                    item_ref: "cursor-item-1".to_owned(),
                    turn_ref: "cursor-turn-1".to_owned(),
                    sequence: 1,
                    kind: HistoryItemKind::UserInput,
                    content_ref: None,
                    redacted: true,
                },
                HistoryItem {
                    item_ref: "cursor-item-2".to_owned(),
                    turn_ref: "cursor-turn-2".to_owned(),
                    sequence: 2,
                    kind: HistoryItemKind::UserInput,
                    content_ref: None,
                    redacted: true,
                },
            ],
            2,
            false,
        )
        .expect("history refresh");
    let first_page = broker
        .history(&binding.binding_id, None, 1)
        .expect("first page");
    let cursor = first_page.next_cursor.clone().expect("next cursor");
    let second_page = broker
        .history(&binding.binding_id, Some(&cursor), 1)
        .expect("second page");
    assert_eq!(second_page.items[0].item_ref, "cursor-item-2");

    let other = broker
        .create_candidate(
            "owner-fixture",
            "other-cursor",
            "branch-other-cursor",
            SessionPurpose::Evaluation,
            support::expiry(),
        )
        .expect("other candidate");
    let other_binding = broker
        .operation(&other.operation_id)
        .expect("other operation")
        .binding_id
        .clone();
    assert_eq!(
        broker.history(&other_binding, Some(&cursor), 1),
        Err(SessionError::Stale)
    );
    assert_eq!(
        broker.history(&binding.binding_id, Some("1:1"), 1),
        Err(SessionError::Stale)
    );
}
