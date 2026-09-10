// SPDX-License-Identifier: MIT

use super::*;
use rusqlite::{Connection, params, types::Value as SqlValue};

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

fn poison_operation_with_digest(
    database: &std::path::Path,
    kind: &str,
    payload: &[u8],
    digest: &str,
) {
    let connection = Connection::open(database).expect("database opens for hostile rewrite");
    connection
        .execute(
            "UPDATE operations SET action_kind = ?1, action_payload = ?2, payload_digest = ?3
             WHERE operation_id = 'operation-hostile'",
            params![kind, SqlValue::Blob(payload.to_vec()), digest],
        )
        .expect("hostile operation row updates");
}

fn poison_catalog(database: &std::path::Path, raw: SqlValue, digest: SqlValue) {
    let connection = Connection::open(database).expect("database opens for catalog rewrite");
    connection
        .execute(
            "UPDATE operations SET catalog_raw = ?1, catalog_digest = ?2
             WHERE operation_id = 'operation-hostile'",
            params![raw, digest],
        )
        .expect("hostile catalog row updates");
}

#[test]
fn legacy_operation_catalog_digest_is_retained_without_inventing_raw_bytes() {
    let database = legacy_operation_database("legacy-catalog-digest");
    let digest = "a".repeat(64);
    poison_catalog(&database, SqlValue::Null, SqlValue::Text(digest.clone()));
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let operation = store
        .operation("operation-hostile")
        .expect("legacy operation remains inspectable");
    assert_eq!(
        operation.intent.catalog_digest.as_deref(),
        Some(digest.as_str())
    );
    assert!(operation.intent.catalog_raw.is_none());
    drop(store);
    remove_database(&database);
}

#[test]
fn operation_reads_reject_stray_malformed_and_oversized_catalog_bytes() {
    let stray = legacy_operation_database("stray-catalog");
    poison_catalog(
        &stray,
        SqlValue::Blob(b"[]".to_vec()),
        SqlValue::Text(result_digest(b"[]")),
    );
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&stray)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&stray);

    let malformed = legacy_operation_database("malformed-catalog");
    let canonical = br#"{"action":{"kind":"end_turn"},"action_id":"end_turn"}"#;
    poison_operation_with_digest(&malformed, "end_turn", canonical, &result_digest(canonical));
    let raw = br#"[}"#;
    poison_catalog(
        &malformed,
        SqlValue::Blob(raw.to_vec()),
        SqlValue::Text(result_digest(raw)),
    );
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&malformed)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&malformed);

    let oversized = legacy_operation_database("oversized-catalog");
    let raw = vec![b'x'; sts2_harness::MAX_CATALOG_BYTES + 1];
    poison_catalog(
        &oversized,
        SqlValue::Blob(raw.clone()),
        SqlValue::Text(result_digest(&raw)),
    );
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&oversized)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&oversized);
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

#[test]
fn operation_reads_reject_matching_digest_malformed_and_hash_mismatch_payloads() {
    let malformed = br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn""#;
    let malformed_database = legacy_operation_database("matching-digest-malformed");
    poison_operation_with_digest(
        &malformed_database,
        "end_turn",
        malformed,
        &result_digest(malformed),
    );
    let store =
        ExecutionStore::open(ExecutionStoreConfig::new(&malformed_database)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&malformed_database);

    let canonical = br#"{"action":{"kind":"end_turn"},"action_id":"end_turn"}"#;
    let mismatch_database = legacy_operation_database("wrong-hash-action");
    poison_operation_with_digest(&mismatch_database, "end_turn", canonical, &"0".repeat(64));
    let store =
        ExecutionStore::open(ExecutionStoreConfig::new(&mismatch_database)).expect("store opens");
    assert!(matches!(
        store.operation("operation-hostile"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&mismatch_database);
}
