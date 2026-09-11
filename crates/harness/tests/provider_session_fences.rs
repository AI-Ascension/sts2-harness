// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::provider_session::*;

#[path = "support/provider_session_fixture.rs"]
mod fixture;

use fixture::{broker, held_binding, item, prepare, prepare_with};

#[test]
fn unapproved_automatic_transform_fences_the_inflight_attempt() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let prepared = prepare(&mut broker, &binding.binding_id, "prepared-transform");
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");
    let operation = broker
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-transform",
        )
        .expect("admit");
    broker
        .mark_sent("owner-fixture", &operation.operation_id)
        .expect("sent");
    let epoch_before = broker
        .binding(&binding.binding_id)
        .expect("binding")
        .history_epoch;

    let event = broker
        .fence_automatic_transform(
            "owner-fixture",
            &binding.binding_id,
            "native-transform-evidence-1",
        )
        .expect("transform fence");
    assert_eq!(event.kind, SessionEventKind::TransformObserved);
    assert_eq!(event.metadata.status, SessionEventStatus::Denied);
    assert!(!event.starts_inference);

    let fenced = broker
        .operation(&operation.operation_id)
        .expect("operation");
    assert_eq!(fenced.state, NativeOperationState::Unknown);
    assert!(!fenced.automatic_retry);
    assert!(!fenced.auto_resume);
    assert_eq!(fenced.game_effects, 0);
    let binding_after = broker.binding(&binding.binding_id).expect("binding");
    assert_eq!(binding_after.state, BindingState::Recovering);
    assert!(!binding_after.game_dispatch_capability);
    assert_eq!(binding_after.history_epoch, epoch_before + 1);

    // A late result for the transformed attempt can never regain authority.
    assert!(
        broker
            .complete_turn(
                "owner-fixture",
                &operation.operation_id,
                "native-transform-evidence-1",
                item(1),
            )
            .is_err()
    );
    // The transform fence is owner-scoped.
    assert_eq!(
        broker.fence_automatic_transform("other-owner", &binding.binding_id, "evidence-x"),
        Err(SessionError::Unauthorized)
    );
}

#[test]
fn clean_rotation_excludes_revoked_state_and_never_resurrects_it() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    broker
        .set_dependencies(
            "owner-fixture",
            &binding.binding_id,
            vec!["source-1".to_owned()],
        )
        .expect("dependencies");
    let prepared = prepare_with(
        &mut broker,
        &binding.binding_id,
        "prepared-rotation",
        vec!["source-1".to_owned()],
    );
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");

    let fork = broker
        .plan_fork(
            "owner-fixture",
            &binding.binding_id,
            "fork-clean-rotation",
            "turn-0",
            0,
            ForkOperation::CleanRehydration,
            vec!["source-1".to_owned()],
        )
        .expect("clean rehydration plan");
    assert!(!fork.copies_native_history);
    assert!(!fork.game_dispatch_capability);

    let retirements = broker
        .revoke_sources("owner-fixture", vec!["source-1".to_owned()])
        .expect("revocation");
    assert_eq!(retirements.len(), 2);
    assert_eq!(
        broker.binding(&binding.binding_id).expect("source").state,
        BindingState::Retired
    );
    // The replacement candidate that inherited the revoked dependency is retired too, so the
    // revoked opaque state cannot be carried into a fresh session.
    assert_eq!(
        broker
            .binding(&fork.target_binding_id)
            .expect("target")
            .state,
        BindingState::Retired
    );
    assert!(
        broker
            .complete_fork("owner-fixture", &fork.fork_plan_id, "native-fork-late")
            .is_err()
    );
    // Prepared input bound to revoked sources cannot be resubmitted.
    assert_eq!(
        broker.admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-revoked",
        ),
        Err(SessionError::Retired)
    );
}

#[test]
fn privacy_serialization_excludes_reasoning_raw_bytes_and_paths() {
    // Unknown/raw fields such as model reasoning are rejected at the record boundary.
    assert!(
        serde_json::from_str::<HistoryItem>(
            r#"{"item_ref":"item","turn_ref":"turn","sequence":1,"kind":"user_input","content_ref":null,"redacted":true,"reasoning":"chain of thought"}"#
        )
        .is_err()
    );
    // Machine paths or secrets cannot be smuggled through an opaque content reference.
    let path_item = HistoryItem {
        item_ref: "item".to_owned(),
        turn_ref: "turn".to_owned(),
        sequence: 1,
        kind: HistoryItemKind::UserInput,
        content_ref: Some("/home/agent/.codex/secret".to_owned()),
        redacted: true,
    };
    assert_eq!(path_item.validate(), Err(SessionError::InvalidRequest));

    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let prepared = prepare(&mut broker, &binding.binding_id, "prepared-privacy");
    let serialized = serde_json::to_string(&prepared).expect("prepared json");
    let prepared_json: serde_json::Value =
        serde_json::from_str(&serialized).expect("prepared value");
    let prepared_object = prepared_json.as_object().expect("prepared object");
    for forbidden in ["suffix", "output_schema", "protected"] {
        assert!(
            !prepared_object.contains_key(forbidden),
            "prepared metadata exposed raw {forbidden} bytes"
        );
    }
    // The raw suffixed input and protected instructions stay inside broker memory.
    assert!(!serialized.contains("exact-suffix-prepared-privacy"));
    assert!(!serialized.contains("\"protected\""));
    assert_eq!(prepared.provider_internal_context, "unexposed");

    broker
        .refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-privacy",
            vec![item(1)],
            1,
            false,
        )
        .expect("history");
    let view = broker.history(&binding.binding_id, None, 8).expect("view");
    let view_json = serde_json::to_value(&view).expect("view json");
    let object = view_json.as_object().expect("object");
    for forbidden in ["reasoning", "raw", "content", "raw_rpc", "secret"] {
        assert!(
            !object.contains_key(forbidden),
            "history projection exposed {forbidden}"
        );
    }
}

#[test]
fn history_cursor_is_opaque_and_does_not_leak_scope_or_binding() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    broker
        .refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-cursor-opaque",
            vec![item(1), item(2)],
            2,
            false,
        )
        .expect("history");
    let view = broker.history(&binding.binding_id, None, 1).expect("page");
    let cursor = view.next_cursor.clone().expect("next cursor");
    for plaintext in [
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
        binding.binding_id.as_str(),
    ] {
        assert!(
            !cursor.contains(plaintext),
            "cursor leaked {plaintext}: {cursor}"
        );
    }
    assert!(cursor.len() <= 64);
    assert!(cursor.split(':').count() == 3);
}
