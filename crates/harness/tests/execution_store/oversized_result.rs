// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use rusqlite::{Connection, params};

use super::*;

#[test]
fn substantially_oversized_result_blob_is_rejected_before_rust_copy() {
    let database = path("oversized-provider-result");
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let current = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let reference = DecisionReference::new(
        current.clone(),
        "execution-oversized",
        "input-oversized",
        "model-revision",
        "provider-config",
    )
    .expect("decision reference is valid");
    store.record_decision(&reference).expect("decision records");
    let reservation = ProviderReservation::new(
        current,
        "reservation-oversized",
        "execution-oversized",
        "provider-execution-oversized",
        1,
    )
    .expect("reservation is valid");
    store
        .reserve_provider(&reservation)
        .expect("reservation records");
    store
        .complete_provider(
            "reservation-oversized",
            "result-oversized",
            "result-digest",
            1,
        )
        .expect("metadata-only completion records");
    store.close().expect("store closes");
    drop(store);

    let connection = Connection::open(&database).expect("SQLite fixture opens");
    connection
        .execute(
            "UPDATE decisions SET result_payload = zeroblob(?1) WHERE execution_id = ?2",
            params![16 * 1024 * 1024_i64, "execution-oversized"],
        )
        .expect("SQLite creates the oversized BLOB without a Rust Vec");
    drop(connection);

    let reopened = ExecutionStore::open_read_only(&database).expect("corrupt fixture opens");
    assert_eq!(
        reopened.decision("execution-oversized"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    );
    drop(reopened);
    remove_database(&database);
}
