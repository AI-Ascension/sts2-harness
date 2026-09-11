// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreError, OperationIntent,
    OperationState, ResumeState,
};

fn lineage() -> ExecutionLineage {
    ExecutionLineage::new(
        "run-phase2",
        "episode-phase2",
        "attempt-phase2",
        "trajectory-phase2",
    )
    .expect("lineage")
}

fn fingerprint() -> ExecutionFingerprint {
    ExecutionFingerprint::new(
        "seed-phase2",
        "build-phase2",
        "state-phase2",
        "config-phase2",
        "provider-phase2",
    )
    .expect("fingerprint")
}

fn operation() -> OperationIntent {
    OperationIntent::new(
        lineage(),
        "operation-phase2",
        "state-phase2",
        1,
        "combat.end-turn",
        "a".repeat(64),
        "b".repeat(64),
    )
    .expect("operation intent")
}

#[test]
fn p2_f061_crash_after_resume_claim_keeps_unknown_and_denies_new_input() {
    let mut store = ExecutionStore::open_in_memory().expect("store");
    let current = lineage();
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    store
        .mark_interrupted_unknown(&current.episode_id, "resume claim interrupted")
        .expect("resume claim interruption is retained");

    assert!(matches!(
        store.resume_episode(&current.episode_id, &fingerprint()),
        Ok(ResumeState::InterruptedUnknown { .. })
    ));
    assert_eq!(
        store.resume_for_decision(&current.episode_id, &fingerprint()),
        Err(ExecutionStoreError::Incompatible)
    );
}

#[test]
fn p2_f062_provider_write_timeout_retains_ambiguous_attempt_without_retransmit() {
    let mut store = ExecutionStore::open_in_memory().expect("store");
    let current = lineage();
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let intent = operation();
    store
        .record_operation_intent(&intent)
        .expect("intent persists");
    store
        .mark_operation_dispatched(&intent.operation_id, &intent.payload_digest)
        .expect("dispatch boundary persists");
    let unknown = store
        .record_operation_result(
            &intent.operation_id,
            &intent.payload_digest,
            OperationState::Unknown,
            Some("unknown-provider-receipt"),
            Some("c".repeat(64).as_str()),
        )
        .expect("unknown provider outcome persists");
    assert_eq!(unknown.state, OperationState::Unknown);
    assert_eq!(
        store.pending_operations(&current.episode_id).unwrap().len(),
        1
    );
    assert_eq!(
        store.mark_operation_dispatched(&intent.operation_id, &intent.payload_digest),
        Err(ExecutionStoreError::Conflict)
    );
}

#[test]
fn p2_f063_game_dispatch_reconciles_the_same_operation_without_a_new_id() {
    let mut store = ExecutionStore::open_in_memory().expect("store");
    let current = lineage();
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    let intent = operation();
    store
        .record_operation_intent(&intent)
        .expect("intent persists");
    store
        .mark_operation_dispatched(&intent.operation_id, &intent.payload_digest)
        .expect("dispatch boundary persists");
    store
        .record_operation_result(
            &intent.operation_id,
            &intent.payload_digest,
            OperationState::Unknown,
            Some("unknown-game-receipt"),
            Some("d".repeat(64).as_str()),
        )
        .expect("unknown game outcome persists");
    let reconciled = store
        .reconcile_operation(
            &intent.operation_id,
            &intent.payload_digest,
            OperationState::Settled,
            "settled-game-receipt",
            &"e".repeat(64),
        )
        .expect("same operation reconciles");
    assert_eq!(reconciled.state, OperationState::Reconciled);
    assert_eq!(reconciled.intent.operation_id, intent.operation_id);
    assert!(
        store
            .pending_operations(&current.episode_id)
            .unwrap()
            .is_empty()
    );
}
