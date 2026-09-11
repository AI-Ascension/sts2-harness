// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn fingerprint_mismatch_and_unknown_attempt_never_start_new_work_implicitly() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-1", "trajectory-1");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    assert!(matches!(
        store.resume_episode(
            "episode-1",
            &ExecutionFingerprint::new("seed-2", "build-1", "state-1", "config-1", "provider-1")
                .expect("alternate fingerprint is valid")
        ),
        Ok(ResumeState::ReconstructionRequired { .. })
    ));
    store
        .mark_interrupted_unknown("episode-1", "MCP connection was lost")
        .expect("unknown attempt is durable");
    assert!(matches!(
        store.resume_episode("episode-1", &fingerprint()),
        Ok(ResumeState::InterruptedUnknown { .. })
    ));
    assert!(matches!(
        store.record_recovery_disposition(
            "episode-1",
            RecoveryDisposition::InPlaceContinuation,
            "awaiting authoritative reconciliation"
        ),
        Ok(attempt) if attempt.state == AttemptState::InterruptedUnknown
    ));
}

#[test]
fn missing_or_empty_state_is_not_recreated_as_a_new_execution_epoch() {
    let missing = path("missing");
    assert!(matches!(
        ExecutionStore::open_read_only(&missing),
        Err(sts2_harness::ExecutionStoreError::Missing)
    ));
    let empty = path("empty");
    fs::File::create(&empty).expect("empty fixture can be created");
    assert!(matches!(
        ExecutionStore::open(ExecutionStoreConfig::new(&empty)),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    remove_database(&empty);
}

#[test]
fn explicit_resume_entrypoint_starts_only_missing_episodes_and_requires_reconciliation() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let current = lineage("attempt-1", "trajectory-1");
    assert!(matches!(
        store.resume_or_start_episode(&current, &fingerprint()),
        Ok(ResumeState::Ready {
            checkpoint: None,
            pending_operations,
            pending_decisions
        }) if pending_operations.is_empty() && pending_decisions.is_empty()
    ));
    assert_eq!(
        store
            .resume_for_decision("episode-1", &fingerprint())
            .expect("empty resume is admitted for a first decision"),
        None
    );
    let intent = OperationIntent::new(
        current,
        "operation-1",
        "state-1",
        1,
        "end_turn",
        "payload",
        "input",
    )
    .expect("operation is valid");
    store
        .record_operation_intent(&intent)
        .expect("intent commits");
    assert!(matches!(
        store.resume_for_decision("episode-1", &fingerprint()),
        Err(sts2_harness::ExecutionStoreError::Conflict)
    ));
}
