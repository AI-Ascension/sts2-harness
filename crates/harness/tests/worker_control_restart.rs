// SPDX-License-Identifier: MIT

use sts2_harness::{
    ExecutionStore, WorkerBoot, WorkerControlMode, WorkerControlRequest, WorkerOwnerProof,
};

#[test]
fn surviving_watchdog_can_authorize_restarted_worker_without_reviving_old_worker()
-> Result<(), Box<dyn std::error::Error>> {
    let mut store = ExecutionStore::open_in_memory()?;
    let profile = "a".repeat(64);
    let watchdog = "11111111-1111-4111-8111-111111111111";
    let worker_before = "22222222-2222-4222-8222-222222222222";
    let worker_after = "33333333-3333-4333-8333-333333333333";
    let proof = WorkerOwnerProof::new("synthetic-authenticated-current-owner")?;
    store.start_worker_boot(&WorkerBoot::new(
        "deployment",
        "worker",
        &profile,
        worker_before,
    )?)?;
    let original_control = WorkerControlRequest::new(
        "deployment",
        "worker",
        &profile,
        watchdog,
        worker_before,
        WorkerControlMode::Running,
        1,
    )?;
    store.set_worker_control_mode(&original_control, &proof)?;
    let stopped = store.start_worker_boot(&WorkerBoot::new(
        "deployment",
        "worker",
        &profile,
        worker_after,
    )?)?;
    assert_eq!(stopped.mode, WorkerControlMode::Stopped);
    assert!(!stopped.authenticated);
    assert!(!stopped.admitting);
    assert!(
        store
            .set_worker_control_mode(&original_control, &proof)
            .is_err()
    );

    let replacement_control = WorkerControlRequest::new(
        "deployment",
        "worker",
        &profile,
        watchdog,
        worker_after,
        WorkerControlMode::Running,
        2,
    )?;
    let resumed = store.set_worker_control_mode(&replacement_control, &proof)?;
    assert!(resumed.authenticated);
    assert!(resumed.admitting);
    assert_eq!(resumed.worker_boot_id, worker_after);
    assert_eq!(resumed.watchdog_boot_id.as_deref(), Some(watchdog));
    assert!(
        store
            .set_worker_control_mode(&original_control, &proof)
            .is_err()
    );
    Ok(())
}
