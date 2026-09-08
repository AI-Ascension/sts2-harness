// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use rusqlite::Connection;
use sts2_harness::{
    CompletionRecord, CompletionStatus, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    ExecutionStoreError, JobState, WorkerCompletionStatus, WorkerHandoffState, WorkerLookup,
    WorkerTerminalReceipt,
};

use super::worker_support::*;

#[test]
fn only_fresh_admission_authorizes_one_worker_execution() {
    let (mut store, tuple, context, permit) = admitted_store_with_permit();
    let running = store
        .mark_worker_handoff_running(permit, &context)
        .expect("fresh permit starts the handoff");
    assert_eq!(running.state, WorkerHandoffState::Running);

    let duplicate = store
        .admit_worker_handoff(&tuple, &context)
        .expect("duplicate admission reads the retained handoff");
    assert_eq!(duplicate.handoff().state, WorkerHandoffState::Running);
    assert!(duplicate.into_acquired().is_none());

    let unknown = store
        .mark_worker_handoff_unknown(&tuple.handoff_id)
        .expect("uncertainty is retained");
    assert_eq!(unknown.state, WorkerHandoffState::Unknown);
    let resumed = store
        .admit_worker_handoff(&tuple, &context)
        .expect("unknown duplicate remains readable");
    assert!(resumed.into_acquired().is_none());
}

#[test]
fn execution_permit_is_bound_to_the_store_incarnation() {
    let (_owner, _tuple, context, permit) = admitted_store_with_permit();
    let (mut other, tuple, _other_context, _other_permit) = admitted_store_with_permit();
    assert!(matches!(
        other.mark_worker_handoff_running(permit, &context),
        Err(ExecutionStoreError::Conflict)
    ));
    assert_eq!(
        other
            .worker_handoff(&tuple.handoff_id)
            .expect("other store reads")
            .expect("other handoff remains")
            .state,
        WorkerHandoffState::Admitted
    );
}

#[test]
fn reopened_store_can_observe_a_duplicate_but_cannot_use_old_permit() {
    let (mut store, tuple, context, permit, database) =
        admitted_file_store_with_permit("permit-reopen");
    store.close().expect("store closes");
    drop(store);

    let mut reopened = ExecutionStore::open(sts2_harness::ExecutionStoreConfig::new(&database))
        .expect("store reopens");
    assert!(matches!(
        reopened.mark_worker_handoff_running(permit, &context),
        Err(ExecutionStoreError::Conflict)
    ));
    let duplicate = reopened
        .admit_worker_handoff(&tuple, &context)
        .expect("reopened duplicate is readable");
    assert!(duplicate.into_acquired().is_none());
    assert_eq!(
        reopened
            .worker_handoff(&tuple.handoff_id)
            .expect("reopened handoff reads")
            .expect("reopened handoff remains")
            .state,
        WorkerHandoffState::Admitted
    );
    drop(reopened);
    remove_database(&database);
}

#[test]
fn terminal_record_projection_rejects_oversized_blobs_and_wrong_storage_types() {
    for (name, statement) in [
        (
            "oversized-terminal",
            "UPDATE worker_handoffs SET terminal_record = zeroblob(16385)",
        ),
        (
            "text-terminal",
            "UPDATE worker_handoffs SET terminal_record = 'not-a-blob'",
        ),
    ] {
        let (mut store, tuple, _context, permit, database) = admitted_file_store_with_permit(name);
        drop(permit);
        store.close().expect("store closes");
        drop(store);
        let connection = Connection::open(&database).expect("database opens");
        connection
            .execute(statement, [])
            .expect("hostile terminal row installs");
        drop(connection);

        let reopened = ExecutionStore::open(sts2_harness::ExecutionStoreConfig::new(&database))
            .expect("hostile database reopens without decoding the row");
        assert!(matches!(
            reopened.worker_handoff(&tuple.handoff_id),
            Err(ExecutionStoreError::Corrupt)
        ));
        drop(reopened);
        remove_database(&database);
    }
}

#[test]
fn legacy_null_terminal_record_remains_a_nonterminal_handoff() {
    let (store, tuple, _context) = admitted_store();
    let handoff = store
        .worker_handoff(&tuple.handoff_id)
        .expect("handoff reads")
        .expect("handoff exists");
    assert!(handoff.terminal.is_none());
    assert_eq!(handoff.state, WorkerHandoffState::Admitted);
}

#[test]
fn failed_worker_completion_retains_failed_job_state() {
    let (mut store, tuple, _context) = admitted_store();
    checkpoint(&mut store);
    let receipt = WorkerTerminalReceipt::new(
        tuple.clone(),
        WorkerCompletionStatus::Failed,
        1,
        "terminal-failed",
        "b".repeat(64),
    )
    .expect("failed receipt is valid");
    let handoff = store
        .record_worker_completion(&receipt)
        .expect("failed completion commits");
    assert_eq!(handoff.state, WorkerHandoffState::Terminal);
    assert_eq!(
        store.job("job-1").expect("job reads").state,
        JobState::Failed
    );
    assert!(matches!(
        store.claim_job("job-1", "replacement-worker"),
        Err(ExecutionStoreError::Conflict)
    ));
}

#[test]
fn terminal_reference_uses_the_wire_utf8_byte_bound_and_roundtrips_ack() {
    for (length, name) in [(513, "terminal-ref-513"), (1024, "terminal-ref-1024")] {
        let (mut store, tuple, _context, permit, database) = admitted_file_store_with_permit(name);
        drop(permit);
        checkpoint(&mut store);
        let terminal_ref = utf8_reference(length);
        let receipt = WorkerTerminalReceipt::new(
            tuple.clone(),
            WorkerCompletionStatus::Completed,
            1,
            terminal_ref.clone(),
            "b".repeat(64),
        )
        .expect("wire-sized terminal reference is accepted");
        let recorded = store
            .record_worker_completion(&receipt)
            .expect("worker completion commits");
        assert_eq!(recorded.terminal, Some(receipt.clone()));
        store.close().expect("store closes");
        drop(store);

        let mut reopened =
            ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store reopens");
        let completion = reopened
            .completion(EPISODE_ID)
            .expect("completion reads")
            .expect("completion persists");
        assert_eq!(completion.terminal_ref, terminal_ref);
        completion.validate().expect("completion remains valid");
        let known = match reopened
            .lookup_worker_handoff(&tuple)
            .expect("worker lookup reads")
        {
            WorkerLookup::Known(value) => *value,
            WorkerLookup::Unknown { .. } => panic!("completed handoff disappeared"),
        };
        assert_eq!(known.terminal, Some(receipt.clone()));
        let digest = receipt.acknowledgment_digest().expect("ack hashes");
        let acknowledged = reopened
            .acknowledge_worker_handoff(&tuple, &digest)
            .expect("acknowledgement commits");
        assert_eq!(acknowledged.state, WorkerHandoffState::Acknowledged);
        assert!(acknowledged.acknowledged);
        drop(reopened);
        remove_database(&database);
    }
    assert!(
        WorkerTerminalReceipt::new(
            tuple(HANDOFF_1, "job-1", "attempt-1"),
            WorkerCompletionStatus::Completed,
            1,
            utf8_reference(1025),
            "b".repeat(64),
        )
        .is_err()
    );
    assert!(
        CompletionRecord::new(
            ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
                .expect("lineage is valid"),
            CompletionStatus::Completed,
            utf8_reference(1025),
            1,
            "b".repeat(64),
        )
        .is_err()
    );
    assert!(
        WorkerTerminalReceipt::new(
            tuple(HANDOFF_1, "job-1", "attempt-1"),
            WorkerCompletionStatus::Completed,
            1,
            "bad\u{001f}reference",
            "b".repeat(64),
        )
        .is_err()
    );
}

fn utf8_reference(target_bytes: usize) -> String {
    let mut value = String::new();
    while value.len() + 2 <= target_bytes {
        value.push('é');
    }
    if value.len() < target_bytes {
        value.push('a');
    }
    assert_eq!(value.len(), target_bytes);
    value
}

#[test]
fn duplicate_terminal_rejects_incompatible_durable_completion() {
    let (mut store, tuple, _context, permit, database) =
        admitted_file_store_with_permit("duplicate-completion-corruption");
    drop(permit);
    checkpoint(&mut store);
    let receipt = WorkerTerminalReceipt::new(
        tuple,
        WorkerCompletionStatus::Completed,
        1,
        "terminal-completed",
        "b".repeat(64),
    )
    .expect("synthetic receipt is valid");
    store
        .record_worker_completion(&receipt)
        .expect("original completion commits");
    store.close().expect("store closes");
    drop(store);
    let connection = Connection::open(&database).expect("database opens");
    connection
        .execute("UPDATE completions SET status = 'quarantined'", [])
        .expect("incompatible durable completion installs");
    drop(connection);
    let mut reopened =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("database reopens");
    let result = reopened.record_worker_completion(&receipt);
    drop(reopened);
    remove_database(&database);
    assert!(
        matches!(result, Err(ExecutionStoreError::Corrupt)),
        "a duplicate must not accept a durable completion that cannot project to its receipt"
    );
}
