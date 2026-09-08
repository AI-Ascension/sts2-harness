// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    Checkpoint, ExecutionFingerprint, ExecutionLineage, ExecutionStore,
    WORKER_EMPTY_PARAMETERS_DIGEST, WorkerAdmissionContext, WorkerBoot, WorkerCompletionStatus,
    WorkerControlMode, WorkerControlRequest, WorkerOwnerProof, WorkerTerminalReceipt, WorkerTuple,
};

pub(super) const RUN_ID: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const EPISODE_ID: &str = "22222222-2222-4222-8222-222222222222";
pub(super) const TRAJECTORY_ID: &str = "33333333-3333-4333-8333-333333333333";
pub(super) const RUN_ID_2: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
pub(super) const EPISODE_ID_2: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
pub(super) const TRAJECTORY_ID_2: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
pub(super) const HANDOFF_1: &str = "44444444-4444-4444-8444-444444444444";
pub(super) const HANDOFF_2: &str = "55555555-5555-4555-8555-555555555555";
pub(super) const HANDOFF_3: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
pub(super) const WORKER_BOOT_1: &str = "66666666-6666-4666-8666-666666666666";
pub(super) const WORKER_BOOT_2: &str = "77777777-7777-4777-8777-777777777777";
pub(super) const WATCHDOG_BOOT_1: &str = "88888888-8888-4888-8888-888888888888";
pub(super) const WATCHDOG_BOOT_2: &str = "99999999-9999-4999-8999-999999999999";

pub(super) fn path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sts2-harness-worker-{name}-{}-{nonce}.sqlite3",
        std::process::id()
    ))
}

pub(super) fn profile() -> String {
    "a".repeat(64)
}

pub(super) fn fingerprint() -> ExecutionFingerprint {
    ExecutionFingerprint::new("seed-1", "build-1", "state-1", "config-1", "provider-1")
        .expect("test fingerprint is valid")
}

pub(super) fn boot(worker: &str) -> WorkerBoot {
    WorkerBoot::new("deployment-1", "harness", profile(), worker).expect("test boot is valid")
}

pub(super) fn context(watchdog: &str, worker: &str, sequence: u64) -> WorkerAdmissionContext {
    WorkerAdmissionContext::new(watchdog, worker, sequence).expect("test context is valid")
}

pub(super) fn tuple(handoff: &str, job: &str, attempt: &str) -> WorkerTuple {
    WorkerTuple::new(
        handoff,
        "deployment-1",
        job,
        attempt,
        1,
        "harness",
        profile(),
        RUN_ID,
        EPISODE_ID,
        TRAJECTORY_ID,
        WORKER_EMPTY_PARAMETERS_DIGEST,
    )
    .expect("test tuple is valid")
}

pub(super) fn start_worker(store: &mut ExecutionStore) -> WorkerAdmissionContext {
    store
        .start_worker_boot(&boot(WORKER_BOOT_1))
        .expect("worker boots");
    let proof = WorkerOwnerProof::new("authenticated-owner").expect("proof is valid");
    let request = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_1,
        WORKER_BOOT_1,
        WorkerControlMode::Running,
        1,
    )
    .expect("control request is valid");
    store
        .set_worker_control_mode(&request, &proof)
        .expect("worker control is running");
    context(WATCHDOG_BOOT_1, WORKER_BOOT_1, 1)
}

pub(super) fn admitted_store() -> (ExecutionStore, WorkerTuple, WorkerAdmissionContext) {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
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
        .expect("worker tuple admits");
    (store, tuple, context)
}

pub(super) fn checkpoint(store: &mut ExecutionStore) {
    let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
        .expect("lineage is valid");
    store
        .save_checkpoint(
            &Checkpoint::new(
                lineage,
                1,
                "state-1",
                1,
                fingerprint(),
                b"{}".to_vec(),
                "catalog-1",
            )
            .expect("checkpoint is valid"),
        )
        .expect("checkpoint saves");
}

pub(super) fn receipt(tuple: &WorkerTuple) -> WorkerTerminalReceipt {
    WorkerTerminalReceipt::new(
        tuple.clone(),
        WorkerCompletionStatus::Completed,
        1,
        "terminal-1",
        "b".repeat(64),
    )
    .expect("receipt is valid")
}

pub(super) fn remove_database(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
}
