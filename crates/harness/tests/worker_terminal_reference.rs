// SPDX-License-Identifier: MIT

use sts2_harness::{
    Checkpoint, ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    WORKER_EMPTY_PARAMETERS_DIGEST, WorkerAdmissionContext, WorkerBoot, WorkerCompletionStatus,
    WorkerControlMode, WorkerControlRequest, WorkerHandoffState, WorkerOwnerProof,
    WorkerTerminalReceipt, WorkerTuple,
};

#[test]
fn wire_sized_terminal_references_commit_lookup_and_acknowledge()
-> Result<(), Box<dyn std::error::Error>> {
    for length in [512, 513, 1024] {
        let database = TestDatabase::new()?;
        let (mut store, tuple) = admitted(database.open()?)?;
        let reference = format!("{}{}", "é".repeat(length / 2), "x".repeat(length % 2));
        assert_eq!(reference.len(), length);
        let receipt = WorkerTerminalReceipt::new(
            tuple.clone(),
            WorkerCompletionStatus::Completed,
            1,
            reference,
            "b".repeat(64),
        )?;
        let committed = store.record_worker_completion(&receipt)?;
        assert_eq!(committed.terminal.as_ref(), Some(&receipt));
        drop(store);
        let mut store = database.open()?;
        let retained = store.worker_handoff(&tuple.handoff_id)?;
        assert_eq!(
            retained.and_then(|handoff| handoff.terminal),
            Some(receipt.clone())
        );
        let acknowledged =
            store.acknowledge_worker_handoff(&tuple, &receipt.acknowledgment_digest()?)?;
        assert_eq!(acknowledged.state, WorkerHandoffState::Acknowledged);
        assert_eq!(acknowledged.terminal.as_ref(), Some(&receipt));
        drop(store);
        let mut store = database.open()?;
        let duplicate_ack =
            store.acknowledge_worker_handoff(&tuple, &receipt.acknowledgment_digest()?)?;
        assert_eq!(duplicate_ack.state, WorkerHandoffState::Acknowledged);
        assert_eq!(duplicate_ack.terminal.as_ref(), Some(&receipt));
    }
    Ok(())
}

fn admitted(
    mut store: ExecutionStore,
) -> Result<(ExecutionStore, WorkerTuple), Box<dyn std::error::Error>> {
    let run = "11111111-1111-4111-8111-111111111111";
    let episode = "22222222-2222-4222-8222-222222222222";
    let trajectory = "33333333-3333-4333-8333-333333333333";
    let handoff = "44444444-4444-4444-8444-444444444444";
    let worker = "55555555-5555-4555-8555-555555555555";
    let watchdog = "66666666-6666-4666-8666-666666666666";
    let fingerprint = ExecutionFingerprint::new("seed", "build", "state", "config", "provider")?;
    let lineage = ExecutionLineage::new(run, episode, "attempt", trajectory)?;
    store.start_episode(&lineage, &fingerprint)?;
    store.save_checkpoint(&Checkpoint::new(
        lineage,
        1,
        "state",
        1,
        fingerprint,
        b"{}".to_vec(),
        "catalog",
    )?)?;
    store.admit_job("job", episode, WORKER_EMPTY_PARAMETERS_DIGEST)?;
    store.start_worker_boot(&WorkerBoot::new(
        "deployment",
        "owner",
        "a".repeat(64),
        worker,
    )?)?;
    store.set_worker_control_mode(
        &WorkerControlRequest::new(
            "deployment",
            "owner",
            "a".repeat(64),
            watchdog,
            worker,
            WorkerControlMode::Running,
            1,
        )?,
        &WorkerOwnerProof::new("synthetic-current-owner")?,
    )?;
    let tuple = WorkerTuple::new(
        handoff,
        "deployment",
        "job",
        "attempt",
        1,
        "owner",
        "a".repeat(64),
        run,
        episode,
        trajectory,
        WORKER_EMPTY_PARAMETERS_DIGEST,
    )?;
    // This test never starts execution; it verifies durable terminal material.
    let _admission =
        store.admit_worker_handoff(&tuple, &WorkerAdmissionContext::new(watchdog, worker, 1)?)?;
    Ok((store, tuple))
}

struct TestDatabase(std::path::PathBuf);

impl TestDatabase {
    fn new() -> std::io::Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "sts2-worker-terminal-reference-{}",
            uuid::Uuid::new_v4(),
        ));
        std::fs::create_dir(&directory)?;
        Ok(Self(directory))
    }

    fn open(&self) -> Result<ExecutionStore, sts2_harness::ExecutionStoreError> {
        ExecutionStore::open(ExecutionStoreConfig::new(self.0.join("state.sqlite3")))
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        for file in ["state.sqlite3", "state.sqlite3-wal", "state.sqlite3-shm"] {
            let _ = std::fs::remove_file(self.0.join(file));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
