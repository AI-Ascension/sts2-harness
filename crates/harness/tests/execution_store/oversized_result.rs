// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use rusqlite::{Connection, params};
use std::path::PathBuf;

use super::*;

const OVERSIZED_BYTES: i64 = 16 * 1024 * 1024;

struct CompletedFixture {
    database: PathBuf,
    lineage: ExecutionLineage,
    reference: DecisionReference,
    reservation_id: String,
    result_ref: String,
    result_digest: String,
}

struct PendingFixture {
    database: PathBuf,
    reference: DecisionReference,
}

#[derive(Clone, Copy)]
enum Corruption {
    OversizedBlob,
    WrongSqliteType,
}

fn completed_fixture(name: &str) -> CompletedFixture {
    let database = path(name);
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let lineage = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&lineage, &fingerprint())
        .expect("episode starts");
    let execution_id = format!("{name}-execution");
    let reference = DecisionReference::new(
        lineage.clone(),
        execution_id.clone(),
        "input-oversized",
        "model-revision",
        "provider-config",
    )
    .expect("decision reference is valid");
    store.record_decision(&reference).expect("decision records");
    let reservation_id = format!("{name}-reservation");
    let reservation = ProviderReservation::new(
        lineage.clone(),
        reservation_id.clone(),
        execution_id,
        format!("{name}-provider-execution"),
        1,
    )
    .expect("reservation is valid");
    store
        .reserve_provider(&reservation)
        .expect("reservation records");
    let payload = br#"{"decision":"wait","rationale":"bounded fixture"}"#;
    let result_ref = format!("{name}-result");
    let result_digest = result_digest(payload);
    store
        .complete_provider_with_result(&reservation_id, &result_ref, &result_digest, payload, 1)
        .expect("provider result records");
    store.close().expect("store closes");
    drop(store);
    CompletedFixture {
        database,
        lineage,
        reference,
        reservation_id,
        result_ref,
        result_digest,
    }
}

fn pending_fixture(name: &str) -> PendingFixture {
    let database = path(name);
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let lineage = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&lineage, &fingerprint())
        .expect("episode starts");
    let reference = DecisionReference::new(
        lineage,
        format!("{name}-execution"),
        "input-pending",
        "model-revision",
        "provider-config",
    )
    .expect("decision reference is valid");
    store.record_decision(&reference).expect("decision records");
    store.close().expect("store closes");
    drop(store);
    PendingFixture {
        database,
        reference,
    }
}

fn corrupt_payload(database: &PathBuf, execution_id: &str, corruption: Corruption) {
    let connection = Connection::open(database).expect("SQLite fixture opens");
    match corruption {
        Corruption::OversizedBlob => connection
            .execute(
                "UPDATE decisions SET result_payload = zeroblob(?1) WHERE execution_id = ?2",
                params![OVERSIZED_BYTES, execution_id],
            )
            .expect("SQLite creates the oversized BLOB without a Rust Vec"),
        Corruption::WrongSqliteType => connection
            .execute(
                "UPDATE decisions SET result_payload = CAST(zeroblob(?1) AS TEXT)
                 WHERE execution_id = ?2",
                params![OVERSIZED_BYTES, execution_id],
            )
            .expect("SQLite writes an oversized wrong payload type"),
    };
    drop(connection);
}

fn assert_corrupt<T>(result: Result<T, sts2_harness::ExecutionStoreError>) {
    assert!(matches!(
        result,
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
}

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

#[test]
fn decision_start_read_rejects_oversized_projection() {
    let fixture = completed_fixture("oversized-start-read");
    corrupt_payload(
        &fixture.database,
        &fixture.reference.execution_id,
        Corruption::OversizedBlob,
    );
    let mut reopened =
        ExecutionStore::open(ExecutionStoreConfig::new(&fixture.database)).expect("store opens");
    assert_corrupt(reopened.record_decision(&fixture.reference));
    drop(reopened);
    remove_database(&fixture.database);
}

#[test]
fn reuse_read_rejects_wrong_sqlite_type() {
    let fixture = completed_fixture("wrong-type-reuse-read");
    corrupt_payload(
        &fixture.database,
        &fixture.reference.execution_id,
        Corruption::WrongSqliteType,
    );
    let reopened =
        ExecutionStore::open_read_only(&fixture.database).expect("read-only store opens");
    assert_corrupt(reopened.reuse_completed_decision(
        &fixture.lineage,
        &fixture.reference.execution_id,
        &fixture.reference.input_fingerprint,
        &fixture.reference.model_revision,
        &fixture.reference.config_digest,
    ));
    drop(reopened);
    remove_database(&fixture.database);
}

#[test]
fn resume_read_rejects_wrong_sqlite_type() {
    let fixture = pending_fixture("wrong-type-resume-read");
    corrupt_payload(
        &fixture.database,
        &fixture.reference.execution_id,
        Corruption::WrongSqliteType,
    );
    let reopened =
        ExecutionStore::open_read_only(&fixture.database).expect("read-only store opens");
    assert_corrupt(reopened.pending_decisions("episode-1"));
    drop(reopened);
    remove_database(&fixture.database);
}

#[test]
fn legacy_null_result_remains_non_replayable_but_readable() {
    let fixture = completed_fixture("legacy-null-result");
    let connection = Connection::open(&fixture.database).expect("SQLite fixture opens");
    connection
        .execute(
            "UPDATE decisions SET result_payload = NULL WHERE execution_id = ?1",
            [&fixture.reference.execution_id],
        )
        .expect("SQLite writes the legacy NULL payload");
    drop(connection);

    let reopened =
        ExecutionStore::open_read_only(&fixture.database).expect("read-only store opens");
    let decision = reopened
        .decision(&fixture.reference.execution_id)
        .expect("legacy decision remains readable");
    assert!(decision.result_payload.is_none());
    let reusable = reopened
        .reuse_completed_decision(
            &fixture.lineage,
            &fixture.reference.execution_id,
            &fixture.reference.input_fingerprint,
            &fixture.reference.model_revision,
            &fixture.reference.config_digest,
        )
        .expect("legacy reuse lookup succeeds")
        .expect("completed metadata remains discoverable");
    assert!(reusable.result_payload.is_none());
    drop(reopened);
    remove_database(&fixture.database);
}

#[test]
fn complete_existing_read_rejects_oversized_projection() {
    let fixture = completed_fixture("oversized-complete-read");
    corrupt_payload(
        &fixture.database,
        &fixture.reference.execution_id,
        Corruption::OversizedBlob,
    );
    let mut reopened =
        ExecutionStore::open(ExecutionStoreConfig::new(&fixture.database)).expect("store opens");
    assert_corrupt(reopened.complete_provider(
        &fixture.reservation_id,
        &fixture.result_ref,
        &fixture.result_digest,
        1,
    ));
    drop(reopened);
    remove_database(&fixture.database);
}
