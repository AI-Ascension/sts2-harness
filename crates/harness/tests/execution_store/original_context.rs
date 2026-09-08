// SPDX-License-Identifier: MIT

use super::*;
use rusqlite::{Connection, params};

const ORIGINAL_CONTEXT_A: &[u8] = br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1}"#;
const ORIGINAL_CONTEXT_B: &[u8] = br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":2,"lease_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","lease_epoch":2}"#;

fn action_intent(
    lineage: ExecutionLineage,
    operation_id: &str,
    original_context_raw: &[u8],
) -> OperationIntent {
    let action_payload = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#;
    let catalog_raw = br#"[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]"#;
    OperationIntent::new_with_action_and_catalog_and_context(
        lineage,
        operation_id,
        "44444444-4444-4444-8444-444444444444",
        1,
        "combat.end-turn",
        "end_turn",
        action_payload.to_vec(),
        result_digest(action_payload),
        "input-digest",
        Some(result_digest(catalog_raw)),
        Some(catalog_raw.to_vec()),
        Some(original_context_raw.to_vec()),
    )
    .expect("action intent is valid")
}

fn start_store(database: &std::path::Path) -> (ExecutionStore, ExecutionLineage) {
    let current = lineage("attempt-original-context", "trajectory-original-context");
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(database)).expect("store opens");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    (store, current)
}

#[test]
fn original_context_survives_close_reopen_and_remains_immutable() {
    let database = path("original-context-reopen");
    let (mut store, current) = start_store(&database);
    let intent = action_intent(current, "operation-original-context", ORIGINAL_CONTEXT_A);
    let recorded = store
        .record_operation_intent(&intent)
        .expect("operation intent persists");
    assert_eq!(
        recorded.intent.original_context_raw.as_deref(),
        Some(ORIGINAL_CONTEXT_A)
    );
    store.close().expect("store closes");
    drop(store);

    let reopened = ExecutionStore::open_read_only(&database).expect("state reopens");
    assert_eq!(
        reopened
            .operation("operation-original-context")
            .expect("operation remains")
            .intent,
        intent
    );
    drop(reopened);
    remove_database(&database);
}

#[test]
fn changing_original_context_for_the_same_operation_is_a_conflict() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-original-conflict", "trajectory-original-conflict");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let first = action_intent(
        current.clone(),
        "operation-original-conflict",
        ORIGINAL_CONTEXT_A,
    );
    let second = action_intent(current, "operation-original-conflict", ORIGINAL_CONTEXT_B);
    store
        .record_operation_intent(&first)
        .expect("first intent persists");
    assert_eq!(
        store.record_operation_intent(&second),
        Err(sts2_harness::ExecutionStoreError::Conflict)
    );
    assert_eq!(
        store
            .operation("operation-original-conflict")
            .expect("original operation remains")
            .intent
            .original_context_raw
            .as_deref(),
        Some(ORIGINAL_CONTEXT_A)
    );
}

#[test]
fn malformed_or_oversized_original_context_is_rejected_before_rust_copy() {
    for (name, raw) in [
        ("malformed-original-context", br#"{"#.to_vec()),
        (
            "oversized-original-context",
            vec![b'x'; sts2_harness::MAX_ORIGINAL_CONTEXT_BYTES + 1],
        ),
    ] {
        let database = path(name);
        let (mut store, current) = start_store(&database);
        let intent = action_intent(current, name, ORIGINAL_CONTEXT_A);
        store
            .record_operation_intent(&intent)
            .expect("operation intent persists");
        store.close().expect("store closes");
        drop(store);
        let connection = Connection::open(&database).expect("database opens for rewrite");
        connection
            .execute(
                "UPDATE operations SET original_context_raw = ?1 WHERE operation_id = ?2",
                params![raw, name],
            )
            .expect("hostile context row updates");
        drop(connection);
        let reopened = ExecutionStore::open_read_only(&database).expect("state reopens");
        assert_eq!(
            reopened.operation(name),
            Err(sts2_harness::ExecutionStoreError::Corrupt)
        );
        drop(reopened);
        remove_database(&database);
    }
}

#[test]
fn public_original_context_mutation_is_revalidated_before_insert() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-original-mutation", "trajectory-original-mutation");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let mut intent = action_intent(current, "operation-original-mutation", ORIGINAL_CONTEXT_A);
    intent.original_context_raw = Some(vec![b'{', b'}']);
    assert_eq!(
        store.record_operation_intent(&intent),
        Err(sts2_harness::ExecutionStoreError::InvalidOperation)
    );
    assert_eq!(
        store
            .operation("operation-original-mutation")
            .expect_err("invalid intent cannot leave a row"),
        sts2_harness::ExecutionStoreError::Missing
    );
}
