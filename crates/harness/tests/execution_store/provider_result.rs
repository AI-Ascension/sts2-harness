// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use sts2_harness::ExecutionStoreError;

use super::*;

fn provider_store(name: &str) -> (ExecutionStore, PathBuf) {
    let database = path(name);
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let lineage = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&lineage, &fingerprint())
        .expect("episode starts");
    let reference = DecisionReference::new(
        lineage.clone(),
        "execution-1",
        "input-1",
        "model-1",
        "config-1",
    )
    .expect("decision reference is valid");
    store.record_decision(&reference).expect("decision records");
    let reservation = ProviderReservation::new(
        lineage,
        "reservation-1",
        "execution-1",
        "provider-execution-1",
        1,
    )
    .expect("reservation is valid");
    store
        .reserve_provider(&reservation)
        .expect("reservation records");
    (store, database)
}

#[test]
fn result_payload_is_bounded_digest_bound_and_tamper_evident() {
    let (mut store, database) = provider_store("provider-result-payload");
    let oversized = vec![b'x'; 8 * 1024 + 1];
    assert_eq!(
        store.complete_provider_with_result(
            "reservation-1",
            "result-1",
            &format!("{:x}", Sha256::digest(&oversized)),
            &oversized,
            1,
        ),
        Err(ExecutionStoreError::InvalidProviderReservation)
    );
    assert_eq!(
        store.complete_provider_with_result("reservation-1", "result-1", &"0".repeat(64), b"{}", 1),
        Err(ExecutionStoreError::InvalidProviderReservation)
    );
    let payload = br#"{"decision":"wait","rationale":"safe"}"#;
    let digest = format!("{:x}", Sha256::digest(payload));
    store
        .complete_provider_with_result("reservation-1", "result-1", &digest, payload, 1)
        .expect("valid result records");
    store.close().expect("store closes");
    drop(store);
    let connection = Connection::open(&database).expect("SQLite fixture opens");
    connection
        .execute(
            "UPDATE decisions SET result_payload = ?1 WHERE execution_id = ?2",
            params![b"tampered".as_slice(), "execution-1"],
        )
        .expect("test tamper writes directly");
    drop(connection);
    let reopened = ExecutionStore::open_read_only(&database).expect("store reopens");
    assert_eq!(
        reopened.decision("execution-1"),
        Err(ExecutionStoreError::Corrupt)
    );
    drop(reopened);
    remove_database(&database);
}

#[test]
fn duplicate_completion_with_conflicting_result_metadata_is_rejected() {
    let (mut store, database) = provider_store("provider-result-dedup");
    let payload = br#"{"decision":"wait","rationale":"safe"}"#;
    let digest = format!("{:x}", Sha256::digest(payload));
    store
        .complete_provider_with_result("reservation-1", "result-1", &digest, payload, 1)
        .expect("first completion records");
    assert_eq!(
        store.complete_provider_with_result("reservation-1", "result-2", &digest, payload, 1),
        Err(ExecutionStoreError::Conflict)
    );
    drop(store);
    remove_database(&database);

    let (mut metadata_store, metadata_database) = provider_store("provider-result-dedup-metadata");
    metadata_store
        .complete_provider("reservation-1", "result-1", "digest-1", 1)
        .expect("metadata-only completion records");
    assert_eq!(
        metadata_store.complete_provider("reservation-1", "result-2", "digest-1", 1),
        Err(ExecutionStoreError::Conflict)
    );
    drop(metadata_store);
    remove_database(&metadata_database);
}
