// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use rusqlite::Connection;
use sts2_harness::{
    CompletionRecord, CompletionStatus, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    ExecutionStoreError, StoredWorkerHandoff, WORKER_EMPTY_PARAMETERS_DIGEST,
    WorkerAdmissionContext, WorkerBoot, WorkerControlMode, WorkerControlRequest,
    WorkerHandoffState, WorkerLookup, WorkerOwnerProof, WorkerReservationState, WorkerTuple,
};

use super::worker_support::*;

#[test]
fn worker_wire_identities_require_canonical_uuid4_and_bounded_component_ids() {
    let mut invalid = tuple(HANDOFF_1, "job-1", "attempt-1");
    invalid.handoff_id = String::from("handoff-1");
    assert_eq!(invalid.validate(), Err(ExecutionStoreError::InvalidJob));
    let mut duplicate = tuple(HANDOFF_1, "job-1", "attempt-1");
    duplicate.episode_id = String::from(RUN_ID);
    assert_eq!(
        duplicate.validate(),
        Err(ExecutionStoreError::InvalidIdentity)
    );
    assert!(WorkerBoot::new("deployment-1", "harness", profile(), "worker-1").is_err());
    assert!(WorkerAdmissionContext::new("watchdog-1", WORKER_BOOT_1, 1).is_err());
    assert!(
        WorkerControlRequest::new(
            "deployment-1",
            "harness",
            profile(),
            WATCHDOG_BOOT_1,
            "worker-1",
            WorkerControlMode::Running,
            1,
        )
        .is_err()
    );
}

#[test]
fn worker_admission_binds_control_to_tuple_deployment_owner_and_profile() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let lineage = ExecutionLineage::new(RUN_ID_2, EPISODE_ID_2, "attempt-2", TRAJECTORY_ID_2)
        .expect("lineage is valid");
    store
        .start_episode(&lineage, &fingerprint())
        .expect("episode starts");
    store
        .admit_job("job-2", EPISODE_ID_2, WORKER_EMPTY_PARAMETERS_DIGEST)
        .expect("job admits");
    let context = start_worker(&mut store);
    let tuple = WorkerTuple::new(
        HANDOFF_3,
        "other-deployment",
        "job-2",
        "attempt-2",
        1,
        "harness",
        profile(),
        RUN_ID_2,
        EPISODE_ID_2,
        TRAJECTORY_ID_2,
        WORKER_EMPTY_PARAMETERS_DIGEST,
    )
    .expect("tuple is structurally valid");
    assert!(matches!(
        store.admit_worker_handoff(&tuple, &context),
        Err(ExecutionStoreError::Conflict)
    ));
}

#[test]
fn worker_tuple_survives_file_reopen_without_replacing_identity() {
    let database = path("reopen");
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("opens");
    let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
        .expect("lineage is valid");
    store
        .start_episode(&lineage, &fingerprint())
        .expect("episode starts");
    store
        .admit_job("job-1", EPISODE_ID, WORKER_EMPTY_PARAMETERS_DIGEST)
        .expect("job admits");
    let context = start_worker(&mut store);
    let original = tuple(HANDOFF_1, "job-1", "attempt-1");
    let (admitted, permit) = store
        .admit_worker_handoff(&original, &context)
        .expect("tuple admits")
        .into_acquired()
        .expect("fresh admission grants a permit");
    drop(permit);
    store.close().expect("closes");
    drop(store);
    let mut reopened = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("reopens");
    assert_eq!(
        reopened.worker_handoff(HANDOFF_1).expect("reads"),
        Some(admitted)
    );
    let lookup = reopened
        .lookup_worker_handoff(&original)
        .expect("lookup reads");
    assert!(matches!(lookup, WorkerLookup::Known(_)));
    assert_eq!(original, tuple(HANDOFF_1, "job-1", "attempt-1"));
    drop(reopened);
    remove_database(&database);
}

#[test]
fn duplicate_tuple_is_idempotent_but_conflicting_or_rebound_identity_is_rejected() {
    let (mut store, original, context) = admitted_store();
    let duplicate = store
        .admit_worker_handoff(&original, &context)
        .expect("duplicate is idempotent");
    assert_eq!(duplicate.handoff().tuple, original);
    assert!(duplicate.into_acquired().is_none());
    let changed = tuple(HANDOFF_1, "job-1", "attempt-2");
    assert!(matches!(
        store.admit_worker_handoff(&changed, &context),
        Err(ExecutionStoreError::Conflict)
    ));
    let rebound = tuple(HANDOFF_2, "job-1", "attempt-1");
    assert!(matches!(
        store.admit_worker_handoff(&rebound, &context),
        Err(ExecutionStoreError::Conflict)
    ));
}

#[test]
fn completion_is_projected_from_existing_episode_and_ack_is_exactly_idempotent() {
    let (mut store, tuple, _) = admitted_store();
    checkpoint(&mut store);
    let completion = CompletionRecord::new(
        ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
            .expect("lineage is valid"),
        CompletionStatus::Completed,
        "terminal-1",
        1,
        "b".repeat(64),
    )
    .expect("completion is valid");
    store
        .record_completion(&completion)
        .expect("episode completion commits");
    let projected = store
        .lookup_worker_handoff(&tuple)
        .expect("lookup projects completion");
    let StoredWorkerHandoff {
        terminal, state, ..
    } = match projected {
        WorkerLookup::Known(value) => *value,
        WorkerLookup::Unknown { .. } => panic!("admitted tuple disappeared"),
    };
    assert_eq!(state, WorkerHandoffState::Terminal);
    assert_eq!(terminal, Some(receipt(&tuple)));
    let canonical_digest = receipt(&tuple)
        .acknowledgment_digest()
        .expect("canonical terminal digest");
    assert_ne!(canonical_digest, "b".repeat(64));
    assert_eq!(
        store.acknowledge_worker_handoff(&tuple, &"b".repeat(64)),
        Err(ExecutionStoreError::Conflict)
    );
    let acknowledged = store
        .acknowledge_worker_handoff(&tuple, &canonical_digest)
        .expect("acknowledgement commits");
    assert_eq!(acknowledged.state, WorkerHandoffState::Acknowledged);
    assert_eq!(
        store
            .acknowledge_worker_handoff(&tuple, &canonical_digest)
            .expect("duplicate acknowledgement is idempotent"),
        acknowledged
    );
    assert_eq!(
        store.acknowledge_worker_handoff(&tuple, &"c".repeat(64)),
        Err(ExecutionStoreError::Conflict)
    );
}

#[test]
fn unknown_reservation_survives_new_worker_boot_and_stopped_lookup() {
    let (mut store, tuple, _) = admitted_store();
    let unknown = store
        .mark_worker_handoff_unknown(HANDOFF_1)
        .expect("unknown state commits");
    assert_eq!(unknown.state, WorkerHandoffState::Unknown);
    assert_eq!(unknown.reservation_state, WorkerReservationState::Unknown);
    store
        .start_worker_boot(&boot(WORKER_BOOT_2))
        .expect("replacement worker boots stopped");
    let proof = WorkerOwnerProof::new("authenticated-owner").expect("proof is valid");
    let paused = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_2,
        WORKER_BOOT_2,
        WorkerControlMode::Paused,
        1,
    )
    .expect("pause is valid");
    store
        .set_worker_control_mode(&paused, &proof)
        .expect("pause commits");
    let lookup = store
        .lookup_worker_handoff(&tuple)
        .expect("lookup works stopped");
    let known = match lookup {
        WorkerLookup::Known(value) => *value,
        WorkerLookup::Unknown { .. } => panic!("known reservation disappeared"),
    };
    assert_eq!(known.state, WorkerHandoffState::Unknown);
    assert_eq!(known.reservation_state, WorkerReservationState::Unknown);
}

#[test]
fn unknown_reservation_projects_a_durable_terminal_without_releasing_the_job() {
    let (mut store, tuple, _) = admitted_store();
    checkpoint(&mut store);
    store
        .mark_worker_handoff_unknown(HANDOFF_1)
        .expect("unknown state commits");
    let completion = CompletionRecord::new(
        ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
            .expect("lineage is valid"),
        CompletionStatus::Completed,
        "terminal-1",
        1,
        "b".repeat(64),
    )
    .expect("completion is valid");
    store
        .record_completion(&completion)
        .expect("episode completion commits");
    let projected = match store
        .lookup_worker_handoff(&tuple)
        .expect("lookup projects completion")
    {
        WorkerLookup::Known(value) => *value,
        WorkerLookup::Unknown { .. } => panic!("unknown reservation disappeared"),
    };
    assert_eq!(projected.state, WorkerHandoffState::Terminal);
    assert_eq!(
        projected.reservation_state,
        WorkerReservationState::Reserved
    );
    let terminal = projected.terminal.expect("terminal is projected");
    let digest = terminal
        .acknowledgment_digest()
        .expect("canonical terminal digest");
    store
        .acknowledge_worker_handoff(&tuple, &digest)
        .expect("acknowledgement commits");
}

#[test]
fn stale_pause_and_control_boot_cannot_reopen_admission() {
    let (mut store, _, _) = admitted_store();
    let proof = WorkerOwnerProof::new("authenticated-owner").expect("proof is valid");
    let pause = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_1,
        WORKER_BOOT_1,
        WorkerControlMode::Paused,
        2,
    )
    .expect("pause is valid");
    store
        .set_worker_control_mode(&pause, &proof)
        .expect("pause commits");
    let stale_running = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_1,
        WORKER_BOOT_1,
        WorkerControlMode::Running,
        1,
    )
    .expect("stale request is structurally valid");
    assert_eq!(
        store.set_worker_control_mode(&stale_running, &proof),
        Err(ExecutionStoreError::Conflict)
    );
    let replacement = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_2,
        WORKER_BOOT_1,
        WorkerControlMode::Running,
        1,
    )
    .expect("replacement is valid");
    store
        .set_worker_control_mode(&replacement, &proof)
        .expect("authenticated replacement commits");
    let stale_newer = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_1,
        WORKER_BOOT_1,
        WorkerControlMode::Running,
        99,
    )
    .expect("stale newer request is structurally valid");
    assert_eq!(
        store.set_worker_control_mode(&stale_newer, &proof),
        Err(ExecutionStoreError::Conflict)
    );
}

#[test]
fn completion_transaction_failure_rolls_back_receipt_and_episode_projection() {
    let database = path("rollback");
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("opens");
    let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
        .expect("lineage is valid");
    store
        .start_episode(&lineage, &fingerprint())
        .expect("episode starts");
    store
        .admit_job("job-1", EPISODE_ID, WORKER_EMPTY_PARAMETERS_DIGEST)
        .expect("job admits");
    let context = start_worker(&mut store);
    let tuple = tuple(HANDOFF_1, "job-1", "attempt-1");
    store
        .admit_worker_handoff(&tuple, &context)
        .expect("tuple admits");
    checkpoint(&mut store);
    store.close().expect("closes before trigger");
    drop(store);
    let connection = Connection::open(&database).expect("database opens");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_worker_completion AFTER INSERT ON completions
             BEGIN SELECT RAISE(ABORT, 'injected completion failure'); END;",
        )
        .expect("failure trigger installs");
    drop(connection);
    let mut reopened = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("reopens");
    assert!(reopened.record_worker_completion(&receipt(&tuple)).is_err());
    assert!(
        reopened
            .completion(EPISODE_ID)
            .expect("completion reads")
            .is_none()
    );
    assert_eq!(
        reopened
            .worker_handoff(HANDOFF_1)
            .expect("handoff reads")
            .expect("handoff remains")
            .state,
        WorkerHandoffState::Admitted
    );
    drop(reopened);
    let connection = Connection::open(&database).expect("database opens for cleanup");
    connection
        .execute_batch("DROP TRIGGER fail_worker_completion")
        .expect("failure trigger drops");
    remove_database(&database);
}
