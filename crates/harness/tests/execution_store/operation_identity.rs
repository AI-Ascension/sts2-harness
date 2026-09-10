// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::{OperationIntent, OperationState};

#[test]
fn operation_intent_is_durable_idempotent_and_unknown_is_reconciled_without_redispatch() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let intent = OperationIntent::new(
        current.clone(),
        "operation-1",
        "state-1",
        4,
        "end_turn",
        "payload-digest",
        "input-digest",
    )
    .expect("operation intent is valid");
    let first = store
        .record_operation_intent(&intent)
        .expect("intent is persisted");
    assert_eq!(first.state, OperationState::IntentRecorded);
    assert_eq!(store.record_operation_intent(&intent), Ok(first.clone()));
    assert_eq!(
        store
            .mark_operation_dispatched("operation-1", "payload-digest")
            .expect("dispatch uncertainty is persisted")
            .state,
        OperationState::MayHaveBeenDispatched
    );
    assert_eq!(
        store
            .record_operation_result(
                "operation-1",
                "payload-digest",
                OperationState::Unknown,
                None,
                None,
            )
            .expect("unknown outcome is retained")
            .state,
        OperationState::Unknown
    );
    assert_eq!(
        store.pending_operations("episode-1").expect("pending list"),
        vec![store.operation("operation-1").expect("operation remains")]
    );
    let reconciled = store
        .reconcile_operation(
            "operation-1",
            "payload-digest",
            OperationState::Settled,
            "receipt-1",
            "receipt-digest",
        )
        .expect("reconciliation uses the same operation identity");
    assert_eq!(reconciled.state, OperationState::Reconciled);
    assert!(
        store
            .pending_operations("episode-1")
            .expect("pending list")
            .is_empty()
    );
    assert_eq!(
        store.reconcile_operation(
            "operation-1",
            "payload-digest",
            OperationState::Settled,
            "receipt-1",
            "receipt-digest",
        ),
        Ok(reconciled.clone())
    );
    assert!(matches!(
        store.mark_operation_dispatched("operation-1", "payload-digest"),
        Err(sts2_harness::ExecutionStoreError::Conflict)
    ));
}

#[test]
fn complete_action_identity_survives_file_store_reopen() {
    let database = path("action-identity");
    let canonical = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#;
    let payload_digest = result_digest(canonical);
    let catalog_raw = br#"[ {"action_id":"combat.end-turn","action":{"kind":"end_turn"}} ]"#;
    let catalog_digest = result_digest(catalog_raw);
    let current = lineage("attempt-action", "trajectory-action");
    let intent = OperationIntent::new_with_action_and_catalog(
        current.clone(),
        "operation-action",
        "state-action",
        1,
        "combat.end-turn",
        "end_turn",
        canonical.to_vec(),
        payload_digest.clone(),
        "input-action",
        Some(catalog_digest.clone()),
        Some(catalog_raw.to_vec()),
    )
    .expect("complete action intent is valid");
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let recorded = store
        .record_operation_intent(&intent)
        .expect("intent is persisted");
    assert_eq!(recorded.intent.action_kind.as_deref(), Some("end_turn"));
    assert_eq!(
        recorded.intent.action_payload.as_deref(),
        Some(canonical.as_slice())
    );
    assert_eq!(recorded.intent.payload_digest, payload_digest);
    assert_eq!(
        recorded.intent.catalog_digest.as_deref(),
        Some(catalog_digest.as_str())
    );
    assert_eq!(
        recorded.intent.catalog_raw.as_deref(),
        Some(catalog_raw.as_slice())
    );
    store.close().expect("store closes");
    drop(store);

    let reopened =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("state reopens");
    assert_eq!(
        reopened
            .operation("operation-action")
            .expect("operation remains")
            .intent,
        intent
    );
    drop(reopened);
    remove_database(&database);
}

#[test]
fn action_identity_rejects_digest_size_and_catalog_mismatches() {
    let canonical = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#;
    let digest = result_digest(canonical);
    let current = lineage("attempt-action", "trajectory-action");
    assert!(
        OperationIntent::new_with_action(
            current.clone(),
            "operation-digest-mismatch",
            "state-action",
            1,
            "combat.end-turn",
            "end_turn",
            canonical.to_vec(),
            "0".repeat(64),
            "input-action",
            Some("a".repeat(64)),
        )
        .is_err()
    );
    assert!(
        OperationIntent::new_with_action(
            current.clone(),
            "operation-catalog-mismatch",
            "state-action",
            1,
            "combat.end-turn",
            "end_turn",
            canonical.to_vec(),
            digest.clone(),
            "input-action",
            Some("A".repeat(64)),
        )
        .is_err()
    );
    assert!(
        OperationIntent::new_with_action(
            current,
            "operation-too-large",
            "state-action",
            1,
            "combat.end-turn",
            "end_turn",
            vec![b'x'; sts2_harness::MAX_OPERATION_ACTION_BYTES + 1],
            digest,
            "input-action",
            Some("a".repeat(64)),
        )
        .is_err()
    );
}

#[test]
fn action_identity_requires_a_bounded_canonical_unique_envelope() {
    let current = lineage("attempt-envelope", "trajectory-envelope");
    let cases: &[&[u8]] = &[
        br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn""#,
        br#"{"action":{"kind":"end_turn","kind":"end_turn"},"action_id":"combat.end-turn"}"#,
        br#"{"action":{"kind":"end_turn"},"action_id":"combat.other"}"#,
        br#"{ "action": {"kind":"end_turn"}, "action_id":"combat.end-turn" }"#,
    ];
    for (index, payload) in cases.iter().enumerate() {
        let digest = result_digest(payload);
        assert!(
            OperationIntent::new_with_action(
                current.clone(),
                format!("operation-envelope-{index}"),
                "state-envelope",
                1,
                "combat.end-turn",
                "end_turn",
                payload.to_vec(),
                digest,
                "input-envelope",
                None,
            )
            .is_err(),
            "hostile action envelope case {index} must be rejected",
        );
    }
}

#[path = "operation_identity_catalog.rs"]
mod catalog;
