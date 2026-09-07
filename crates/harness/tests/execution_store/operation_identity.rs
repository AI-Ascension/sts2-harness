// SPDX-License-Identifier: MIT

use super::*;
use rusqlite::{Connection, params, types::Value as SqlValue};
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
    let catalog_digest =
        result_digest(br#"[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]"#);
    let current = lineage("attempt-action", "trajectory-action");
    let intent = OperationIntent::new_with_action(
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

fn legacy_operation_database(name: &str) -> std::path::PathBuf {
    let database = path(name);
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let current = lineage("attempt-hostile", "trajectory-hostile");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let intent = OperationIntent::new(
        current,
        "operation-hostile",
        "state-hostile",
        1,
        "end_turn",
        "payload-digest",
        "input-digest",
    )
    .expect("legacy operation is valid");
    store
        .record_operation_intent(&intent)
        .expect("legacy operation is persisted");
    store.close().expect("store closes");
    database
}

fn poison_operation(database: &std::path::Path, kind: Option<&str>, payload: SqlValue) {
    let connection = Connection::open(database).expect("database opens for hostile rewrite");
    connection
        .execute(
            "UPDATE operations SET action_kind = ?1, action_payload = ?2
             WHERE operation_id = 'operation-hostile'",
            params![kind, payload],
        )
        .expect("hostile operation row updates");
}

#[test]
fn operation_reads_reject_oversized_and_malformed_payloads_before_materializing_them() {
    let oversized = legacy_operation_database("oversized-operation");
    poison_operation(
        &oversized,
        Some("end_turn"),
        SqlValue::Blob(vec![b'x'; sts2_harness::MAX_OPERATION_ACTION_BYTES + 1]),
    );
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&oversized)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&oversized);

    let text = legacy_operation_database("text-operation");
    poison_operation(
        &text,
        Some("end_turn"),
        SqlValue::Text(String::from("not-a-blob")),
    );
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&text)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&text);
}

#[test]
fn operation_reads_reject_partial_legacy_action_identity_rows() {
    for (name, kind, payload) in [
        ("kind-without-payload", Some("end_turn"), SqlValue::Null),
        (
            "payload-without-kind",
            None,
            SqlValue::Blob(vec![b'{', b'}']),
        ),
    ] {
        let database = legacy_operation_database(name);
        poison_operation(&database, kind, payload);
        let store =
            ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
        assert!(matches!(
            store.operation("operation-hostile"),
            Err(sts2_harness::ExecutionStoreError::Corrupt)
        ));
        drop(store);
        remove_database(&database);
    }
}
