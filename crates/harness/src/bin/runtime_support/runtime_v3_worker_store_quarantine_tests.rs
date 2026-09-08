// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use serde_json::json;
use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig, ResumeState,
};

use super::super::worker_store::{
    SharedExecutionStore, share_store, try_lock, try_lock_close, try_lock_recovery,
};
use super::{
    DurableHandle, checkpoint_observation, executable_path, fingerprint, lineage, setup_store,
};

const SECOND_RUN_ID: &str = "99999999-9999-4999-8999-999999999999";
const SECOND_EPISODE_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const SECOND_TRAJECTORY_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

fn second_lineage() -> ExecutionLineage {
    ExecutionLineage::new(
        SECOND_RUN_ID,
        SECOND_EPISODE_ID,
        "attempt-2",
        SECOND_TRAJECTORY_ID,
    )
    .expect("second lineage is valid")
}

fn second_fingerprint() -> ExecutionFingerprint {
    ExecutionFingerprint::new("seed-2", "build-2", "state-2", "config-2", "provider-2")
        .expect("second fingerprint is valid")
}

fn two_active_episodes() -> (SharedExecutionStore, DurableHandle, DurableHandle) {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    store
        .start_episode(&lineage(), &fingerprint())
        .expect("first episode starts");
    store
        .start_episode(&second_lineage(), &second_fingerprint())
        .expect("second episode starts");
    let shared = share_store(store);
    let first = DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
        .expect("first runtime attachment");
    let second = DurableHandle::from_shared_store_for_test(
        shared.clone(),
        second_lineage(),
        second_fingerprint(),
    )
    .expect("second runtime attachment");
    (shared, first, second)
}

#[test]
fn failed_quarantine_latches_clones_and_new_handles_but_keeps_diagnostics_and_close() {
    let (store, _tuple, _context) = setup_store();
    let shared = share_store(store);
    let handle =
        DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
            .expect("runtime attachment");
    let lease = try_lock_recovery(&shared).expect("test owns the recovery lease");
    let error = handle
        .mark_interrupted_unknown("contention quarantine")
        .expect_err("contended quarantine must fail closed");
    assert!(error.contains("store is busy"), "unexpected error: {error}");
    drop(lease);

    let cloned_handle = handle.clone();
    let blocked = cloned_handle
        .checkpoint(&checkpoint_observation("state-1", 1), &json!([]))
        .expect_err("failed quarantine must block a cloned handle");
    assert!(
        blocked.contains("fail-closed"),
        "unexpected error: {blocked}"
    );
    assert!(
        try_lock(&shared).is_err(),
        "admission lease bypassed the latch"
    );

    let new_handle =
        DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
            .expect("read-only attachment remains available");
    assert!(new_handle.pending_operations().is_ok());
    let blocked_new_handle = new_handle
        .checkpoint(&checkpoint_observation("state-2", 2), &json!([]))
        .expect_err("failed quarantine must block a new handle");
    assert!(
        blocked_new_handle.contains("fail-closed"),
        "unexpected error: {blocked_new_handle}"
    );

    try_lock_close(&shared)
        .expect("close bypasses the admission latch")
        .close()
        .expect("store closes after a failed quarantine");
}

#[test]
fn underlying_quarantine_error_latches_without_reopening_admission() {
    let (store, _tuple, _context) = setup_store();
    let shared = share_store(store);
    let handle =
        DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
            .expect("runtime attachment");
    try_lock_recovery(&shared)
        .expect("test owns the store lease")
        .close()
        .expect("underlying store closes");

    let error = handle
        .mark_interrupted_unknown("closed-store quarantine")
        .expect_err("closed store quarantine must report its failure");
    assert!(
        error.contains("cannot persist runtime-v3 interrupted-unknown quarantine"),
        "unexpected error: {error}"
    );
    let blocked = handle
        .checkpoint(&checkpoint_observation("state-1", 1), &json!([]))
        .expect_err("underlying quarantine failure must block admission");
    assert!(
        blocked.contains("fail-closed"),
        "unexpected error: {blocked}"
    );
}

#[test]
fn successful_quarantine_persists_unknown_and_keeps_shared_admission_closed() {
    let (store, _tuple, _context) = setup_store();
    let shared = share_store(store);
    let handle =
        DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
            .expect("runtime attachment");
    handle
        .mark_interrupted_unknown("successful quarantine")
        .expect("quarantine persists");

    let state = try_lock_recovery(&shared)
        .expect("read-only diagnostics remain available")
        .resume_episode(super::EPISODE_ID, &fingerprint())
        .expect("resume state reads");
    assert!(matches!(state, ResumeState::InterruptedUnknown { .. }));
    let new_handle = DurableHandle::from_shared_store_for_test(shared, lineage(), fingerprint())
        .expect("read-only reattachment remains available");
    let blocked = new_handle
        .checkpoint(&checkpoint_observation("state-1", 1), &json!([]))
        .expect_err("successful quarantine must keep admission closed");
    assert!(
        blocked.contains("fail-closed"),
        "unexpected error: {blocked}"
    );
}

#[test]
fn successful_quarantine_survives_store_reopen() {
    let path = executable_path("quarantine-reopen");
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(&path)).expect("store opens");
    store
        .start_episode(&lineage(), &fingerprint())
        .expect("episode starts");
    let shared = share_store(store);
    let handle = DurableHandle::from_shared_store_for_test(shared, lineage(), fingerprint())
        .expect("runtime attachment");
    handle
        .mark_interrupted_unknown("persistent quarantine")
        .expect("quarantine persists");
    drop(handle);

    let reopened = ExecutionStore::open(ExecutionStoreConfig::new(&path)).expect("store reopens");
    let state = reopened
        .resume_episode(super::EPISODE_ID, &fingerprint())
        .expect("reopened resume state reads");
    assert!(matches!(state, ResumeState::InterruptedUnknown { .. }));
    drop(reopened);
    std::fs::remove_file(path).expect("quarantine fixture is removed");
}

#[test]
fn quarantine_latch_does_not_acknowledge_a_different_episode() {
    let (shared, first, second) = two_active_episodes();
    first
        .mark_interrupted_unknown("first episode quarantine")
        .expect("first episode quarantine persists");
    second
        .mark_interrupted_unknown("second episode quarantine")
        .expect("second episode quarantine persists despite shared latch");

    let first_state = try_lock_recovery(&shared)
        .expect("first recovery read")
        .resume_episode(super::EPISODE_ID, &fingerprint())
        .expect("first state reads");
    assert!(matches!(
        first_state,
        ResumeState::InterruptedUnknown { .. }
    ));
    let second_state = try_lock_recovery(&shared)
        .expect("second recovery read")
        .resume_episode(SECOND_EPISODE_ID, &second_fingerprint())
        .expect("second state reads");
    assert!(matches!(
        second_state,
        ResumeState::InterruptedUnknown { .. }
    ));
}
