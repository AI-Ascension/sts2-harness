// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    AttemptKind, AttemptState, Checkpoint, CompletionRecord, CompletionStatus, DecisionReference,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig, JobClaimOutcome,
    JobState, OperationIntent, OperationState, ProviderFailureClass, ProviderReservation,
    ProviderReservationState, RECOVERY_SCHEMA_DIGEST, RecoveryDisposition, ResumeState,
};

fn path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sts2-harness-execution-{name}-{}-{nonce}.sqlite3",
        std::process::id()
    ))
}

fn fingerprint() -> ExecutionFingerprint {
    ExecutionFingerprint::new("seed-1", "build-1", "state-1", "config-1", "provider-1")
        .expect("test fingerprint is valid")
}

fn lineage(attempt: &str, trajectory: &str) -> ExecutionLineage {
    ExecutionLineage::new("run-1", "episode-1", attempt, trajectory).expect("test lineage is valid")
}

fn checkpoint(lineage: ExecutionLineage, sequence: u64, generation: u64) -> Checkpoint {
    Checkpoint::new(
        lineage,
        sequence,
        format!("state-{generation}"),
        generation,
        fingerprint(),
        vec![b'{', b'}'],
        "catalog-digest",
    )
    .expect("test checkpoint is valid")
}

#[test]
fn sqlite_store_is_wal_full_and_survives_reopen_without_resetting_state() {
    let database = path("pragmas");
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    let pragmas = store.pragmas().expect("pragmas are readable");
    assert_eq!(pragmas.journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(pragmas.synchronous, 2);
    assert!(pragmas.foreign_keys);
    let first = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&first, &fingerprint())
        .expect("episode starts");
    store.integrity_check().expect("fresh store is intact");
    store.close().expect("store closes");
    drop(store);

    let reopened = ExecutionStore::open_read_only(&database).expect("state reopens read-only");
    let episode = reopened
        .load_episode("episode-1")
        .expect("episode remains present");
    assert_eq!(episode.lineage, first);
    assert_eq!(episode.state, AttemptState::Active);
    reopened
        .integrity_check()
        .expect("reopened store is intact");
    let mut reopened = reopened;
    reopened
        .close()
        .expect("read-only close does not checkpoint");
    drop(reopened);
    remove_database(&database);
}

#[test]
fn default_store_configuration_pins_the_published_recovery_schema() {
    assert_eq!(
        ExecutionStoreConfig::default()
            .recovery_schema_digest
            .as_deref(),
        Some(RECOVERY_SCHEMA_DIGEST)
    );
    assert_eq!(
        RECOVERY_SCHEMA_DIGEST,
        "fb934d3157485aaf6e13e6ebbb213ec8a14c7fc6f5eeebc06b7a22c1f0009217"
    );
}

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
fn provider_reservation_is_conservative_and_completed_decisions_are_reusable() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let reference = DecisionReference::new(
        current.clone(),
        "execution-1",
        "input-fingerprint",
        "model-revision",
        "provider-config",
    )
    .expect("decision reference is valid");
    store
        .record_decision(&reference)
        .expect("decision is durable");
    let reservation = ProviderReservation::new(
        current,
        "reservation-1",
        "execution-1",
        "provider-execution-1",
        10,
    )
    .expect("provider reservation is valid");
    assert_eq!(
        store
            .reserve_provider(&reservation)
            .expect("reservation is durable")
            .state,
        ProviderReservationState::Reserved
    );
    let completed = store
        .complete_provider("reservation-1", "result-1", "result-digest", 7)
        .expect("provider result is durable");
    assert!(completed.completed);
    assert!(!completed.unknown);
    let reusable = store
        .reuse_completed_decision(
            "episode-1",
            "input-fingerprint",
            "model-revision",
            "provider-config",
        )
        .expect("reuse lookup succeeds")
        .expect("completed result is reusable");
    assert_eq!(reusable.reference.execution_id, "execution-1");

    let unknown_reference = DecisionReference::new(
        lineage("attempt-1", "trajectory-1"),
        "execution-2",
        "input-fingerprint-2",
        "model-revision",
        "provider-config",
    )
    .expect("second decision reference is valid");
    store
        .record_decision(&unknown_reference)
        .expect("second decision is durable");
    let unknown_reservation = ProviderReservation::new(
        lineage("attempt-1", "trajectory-1"),
        "reservation-2",
        "execution-2",
        "provider-execution-2",
        5,
    )
    .expect("second reservation is valid");
    store
        .reserve_provider(&unknown_reservation)
        .expect("second reservation is durable");
    let unknown = store
        .mark_provider_unknown("reservation-2", ProviderFailureClass::Timeout, None)
        .expect("ambiguous provider use is retained");
    assert!(unknown.unknown);
    assert!(
        store
            .reuse_completed_decision(
                "episode-1",
                "input-fingerprint-2",
                "model-revision",
                "provider-config",
            )
            .expect("unknown reuse lookup succeeds")
            .is_none()
    );
}

#[test]
fn checkpoint_reconstruction_copies_the_verified_boundary_and_preserves_attempt_history() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let old = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&old, &fingerprint())
        .expect("episode starts");
    let saved = checkpoint(old.clone(), 3, 12);
    assert!(store.save_checkpoint(&saved).expect("checkpoint saves"));
    assert!(
        !store
            .save_checkpoint(&saved)
            .expect("identical checkpoint is idempotent")
    );
    let next = lineage("attempt-2", "trajectory-2");
    let attempt = store
        .reconstruct_attempt(&saved, &next, &fingerprint(), "replay-prefix-1")
        .expect("reconstruction starts from the approved checkpoint");
    assert_eq!(attempt.kind, AttemptKind::Reconstruction);
    assert_eq!(attempt.parent_attempt_id.as_deref(), Some("attempt-1"));
    assert_eq!(attempt.state, AttemptState::Active);
    assert_eq!(
        store
            .attempts_for_episode("episode-1")
            .expect("attempt history loads")
            .len(),
        2
    );
    let resumed = store
        .load_episode("episode-1")
        .expect("current episode loads");
    assert_eq!(resumed.lineage, next);
    assert_eq!(
        resumed.last_checkpoint,
        Some(checkpoint(next.clone(), 3, 12))
    );
    assert_eq!(
        store.attempt("attempt-1").expect("old attempt loads").state,
        AttemptState::Failed
    );
    assert_eq!(
        store.resume_episode("episode-1", &fingerprint()),
        Ok(ResumeState::Ready {
            checkpoint: Some(checkpoint(next, 3, 12)),
            pending_operations: Vec::new(),
            pending_decisions: Vec::new(),
        })
    );
}

#[test]
fn completion_and_job_claim_are_durable_before_acknowledgement() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let saved = checkpoint(current.clone(), 1, 1);
    store.save_checkpoint(&saved).expect("checkpoint saves");
    let completion = CompletionRecord::new(
        current,
        CompletionStatus::Completed,
        "terminal-1",
        1,
        "terminal-digest",
    )
    .expect("completion is valid");
    assert!(
        store
            .record_completion(&completion)
            .expect("completion commits")
    );
    assert!(
        !store
            .record_completion(&completion)
            .expect("completion retry is idempotent")
    );
    assert_eq!(
        store.resume_episode("episode-1", &fingerprint()),
        Ok(ResumeState::Completed(completion))
    );

    let admitted = store
        .admit_job("job-1", "episode-1", "job-payload")
        .expect("job admission commits");
    assert_eq!(admitted.state, JobState::Admitted);
    let claim = match store
        .claim_job_with_token("job-1", "worker-1", "claim-1")
        .expect("job claim commits")
    {
        JobClaimOutcome::Claimed(claim) => claim,
        other => panic!("unexpected claim result: {other:?}"),
    };
    let completed = store
        .complete_job("job-1", &claim.claim_token, "job-result")
        .expect("job completion commits");
    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(
        store
            .acknowledge_job("job-1")
            .expect("ack reads durable completion")
            .state,
        JobState::Completed
    );
    assert!(matches!(
        store.claim_job("job-1", "worker-2"),
        Ok(JobClaimOutcome::AlreadyCompleted(_))
    ));
}

#[path = "execution_store/recovery.rs"]
mod recovery;

fn remove_database(database: &PathBuf) {
    let _ = fs::remove_file(database);
    let _ = fs::remove_file(database.with_extension("sqlite3-wal"));
    let _ = fs::remove_file(database.with_extension("sqlite3-shm"));
}
