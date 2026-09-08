// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use sts2_harness::{
    EpisodeObservation, EpisodeRunnerConfig, EpisodeStage, ExecutionFingerprint, ExecutionLineage,
    ExecutionStore, ExoConfig, ExoProcessConfig, RecoveryController, StabilityBarrier,
    StoredWorkerHandoff, WORKER_EMPTY_PARAMETERS_DIGEST, WorkerAdmissionContext, WorkerBoot,
    WorkerControlMode, WorkerControlRequest, WorkerOwnerProof, WorkerTuple,
};

use super::super::runtime_v3_settings::RuntimeV3Settings;
use super::durable::DurableHandle;
use super::worker_store::{SharedExecutionStore, share_store, try_lock};

#[path = "runtime_v3_worker_store_attachment_tests.rs"]
mod attachment_tests;
#[path = "runtime_v3_worker_store_fence_tests.rs"]
mod fence_tests;
#[path = "runtime_v3_worker_store_quarantine_tests.rs"]
mod quarantine_tests;

const RUN_ID: &str = "11111111-1111-4111-8111-111111111111";
const EPISODE_ID: &str = "22222222-2222-4222-8222-222222222222";
const TRAJECTORY_ID: &str = "33333333-3333-4333-8333-333333333333";
const HANDOFF_ID: &str = "44444444-4444-4444-8444-444444444444";
const WORKER_BOOT_ID: &str = "66666666-6666-4666-8666-666666666666";
const WATCHDOG_BOOT_ID: &str = "88888888-8888-4888-8888-888888888888";

fn runtime_config(mcp_binary: &str) -> super::super::config::RuntimeConfig {
    super::super::config::RuntimeConfig {
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: mcp_binary.to_owned(),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("gateway-session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-independent"),
        run_id: RUN_ID.into(),
        episode_id: EPISODE_ID.into(),
        trajectory_id: TRAJECTORY_ID.into(),
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        recovery_environment: Vec::new(),
    }
}

fn runtime_settings(binary: &str) -> RuntimeV3Settings {
    RuntimeV3Settings {
        runner: EpisodeRunnerConfig::new(
            1,
            StabilityBarrier::new(1, 1).expect("barrier"),
            RecoveryController::new(1).expect("recovery"),
            "objective",
            Vec::new(),
        )
        .expect("runner"),
        exo: ExoConfig::new("a".repeat(64), 1024, 1024, 1).expect("exo"),
        process: ExoProcessConfig::new(binary, Vec::new(), None, Vec::new()).expect("process"),
    }
}

fn executable_path(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sts2-worker-store-{name}-{}-{nonce}",
        std::process::id()
    ))
}

fn approved_fingerprint_for(
    config: &super::super::config::RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> ExecutionFingerprint {
    let config_digest = super::durable::config_digest_for_test(config, settings).expect("digest");
    ExecutionFingerprint::new(
        "seed-1",
        "build-1",
        "state-1",
        config_digest,
        "a".repeat(64),
    )
    .expect("approved fingerprint")
}

fn lineage() -> ExecutionLineage {
    ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID).expect("valid lineage")
}

fn fingerprint() -> ExecutionFingerprint {
    ExecutionFingerprint::new("seed-1", "build-1", "state-1", "config-1", "provider-1")
        .expect("valid fingerprint")
}

fn setup_store() -> (ExecutionStore, WorkerTuple, WorkerAdmissionContext) {
    setup_store_with_fingerprint(&fingerprint())
}

fn setup_store_with_fingerprint(
    approved_fingerprint: &ExecutionFingerprint,
) -> (ExecutionStore, WorkerTuple, WorkerAdmissionContext) {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    store
        .start_episode(&lineage(), approved_fingerprint)
        .expect("episode starts");
    store
        .admit_job("job-1", EPISODE_ID, WORKER_EMPTY_PARAMETERS_DIGEST)
        .expect("job admits");
    let profile = "a".repeat(64);
    let boot = WorkerBoot::new("deployment-1", "harness", profile.clone(), WORKER_BOOT_ID)
        .expect("worker boot");
    store.start_worker_boot(&boot).expect("worker starts");
    let request = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile.clone(),
        WATCHDOG_BOOT_ID,
        WORKER_BOOT_ID,
        WorkerControlMode::Running,
        1,
    )
    .expect("control request");
    let proof = WorkerOwnerProof::new("owner-proof").expect("owner proof");
    store
        .set_worker_control_mode(&request, &proof)
        .expect("control starts");
    let context = WorkerAdmissionContext::new(WATCHDOG_BOOT_ID, WORKER_BOOT_ID, 1)
        .expect("admission context");
    let tuple = WorkerTuple::new(
        HANDOFF_ID,
        "deployment-1",
        "job-1",
        "attempt-1",
        1,
        "harness",
        profile,
        RUN_ID,
        EPISODE_ID,
        TRAJECTORY_ID,
        WORKER_EMPTY_PARAMETERS_DIGEST,
    )
    .expect("worker tuple");
    (store, tuple, context)
}

fn prepared_runtime() -> (
    std::path::PathBuf,
    super::super::config::RuntimeConfig,
    RuntimeV3Settings,
    ExecutionFingerprint,
    SharedExecutionStore,
    StoredWorkerHandoff,
) {
    let path = executable_path("bridge");
    std::fs::write(&path, b"synthetic bridge").expect("bridge fixture");
    let path_text = path.to_str().expect("path is UTF-8");
    let config = runtime_config(path_text);
    let settings = runtime_settings(path_text);
    let approved = approved_fingerprint_for(&config, &settings);
    let (mut store, tuple, context) = setup_store_with_fingerprint(&approved);
    let permit = store
        .admit_worker_handoff(&tuple, &context)
        .expect("handoff admits")
        .into_acquired()
        .expect("fresh handoff has permit")
        .1;
    let handoff = store
        .mark_worker_handoff_running(permit, &context)
        .expect("handoff runs");
    (
        path,
        config,
        settings,
        approved,
        share_store(store),
        handoff,
    )
}

fn remove_executable(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

fn foreign_lineage() -> ExecutionLineage {
    ExecutionLineage::new(
        "foreign-run",
        EPISODE_ID,
        "foreign-attempt",
        "foreign-trajectory",
    )
    .expect("foreign lineage is valid")
}

fn foreign_fingerprint() -> ExecutionFingerprint {
    ExecutionFingerprint::new(
        "foreign-seed",
        "foreign-build",
        "foreign-state",
        "config-1",
        "provider-1",
    )
    .expect("foreign fingerprint is valid")
}

fn checkpoint_observation(state_id: &str, generation: u64) -> EpisodeObservation {
    EpisodeObservation::new(
        state_id,
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        json!({
            "state_id": state_id,
            "generation": generation,
            "player": {
                "hp": 50,
                "max_hp": 50,
                "energy": 3,
                "gold": 99,
                "hand": [],
                "deck": [],
                "discard": [],
                "exhaust": []
            },
            "state": {"state": "combat", "turn_index": 1, "enemies": []},
            "legal_actions": []
        }),
    )
    .expect("checkpoint observation is valid")
}

#[test]
fn shared_store_is_send_sync_and_cross_thread_operations_use_one_connection() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SharedExecutionStore>();

    let (mut store, tuple, context) = setup_store();
    let permit = store
        .admit_worker_handoff(&tuple, &context)
        .expect("handoff admits")
        .into_acquired()
        .expect("fresh handoff has permit")
        .1;
    let shared = share_store(store);
    let worker_store = shared.clone();
    let running = thread::spawn(move || {
        let mut store = try_lock(&worker_store).expect("worker obtains short store lease");
        store
            .mark_worker_handoff_running(permit, &context)
            .expect("handoff becomes running")
    })
    .join()
    .expect("worker thread joins");
    assert_eq!(running.tuple, tuple);
    let control = try_lock(&shared)
        .expect("control obtains short lease")
        .worker_control()
        .expect("control reads")
        .expect("control exists");
    assert_eq!(control.mode, WorkerControlMode::Running);
    assert_eq!(control.worker_boot_id, WORKER_BOOT_ID);
    assert!(control.admitting);
}

#[test]
fn server_owned_shared_store_survives_runtime_handle_close() {
    let (mut store, tuple, context) = setup_store();
    store
        .admit_worker_handoff(&tuple, &context)
        .expect("handoff admits");
    let shared = share_store(store);
    let handle =
        DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
            .expect("runtime attaches to shared store");
    handle.close().expect("server-owned close is a no-op");
    assert!(
        try_lock(&shared)
            .expect("server retains store ownership")
            .integrity_check()
            .is_ok()
    );
}

#[test]
fn shared_runtime_attachment_rejects_a_foreign_stored_lineage() {
    let (store, _tuple, _context) = setup_store();
    let result = DurableHandle::from_shared_store_for_test(
        share_store(store),
        foreign_lineage(),
        fingerprint(),
    );
    let error = match result {
        Ok(_) => panic!("foreign stored lineage was attached"),
        Err(error) => error,
    };
    assert!(
        error.contains("stored episode lineage"),
        "unexpected error: {error}"
    );
}

#[test]
fn shared_runtime_attachment_rejects_a_foreign_stored_fingerprint() {
    let (store, _tuple, _context) = setup_store();
    let result = DurableHandle::from_shared_store_for_test(
        share_store(store),
        lineage(),
        foreign_fingerprint(),
    );
    let error = match result {
        Ok(_) => panic!("foreign stored fingerprint was attached"),
        Err(error) => error,
    };
    assert!(
        error.contains("stored episode fingerprint"),
        "unexpected error: {error}"
    );
}

#[test]
fn shared_runtime_attachment_reports_store_contention_without_waiting() {
    let (store, _tuple, _context) = setup_store();
    let shared = share_store(store);
    let guard = try_lock(&shared).expect("test owns the store lease");
    let result =
        DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint());
    drop(guard);
    let error = match result {
        Ok(_) => panic!("contended shared-store attachment succeeded"),
        Err(error) => error,
    };
    assert!(error.contains("store is busy"), "unexpected error: {error}");
}

#[test]
fn stale_shared_runtime_checkpoint_is_rejected_before_a_duplicate_sequence() {
    let (store, _tuple, _context) = setup_store();
    let shared = share_store(store);
    let first = DurableHandle::from_shared_store_for_test(shared.clone(), lineage(), fingerprint())
        .expect("first runtime attachment");
    let second = DurableHandle::from_shared_store_for_test(shared, lineage(), fingerprint())
        .expect("second runtime attachment");
    first
        .checkpoint(&checkpoint_observation("state-1", 1), &json!([]))
        .expect("first checkpoint persists");
    let error = match second.checkpoint(&checkpoint_observation("state-2", 2), &json!([])) {
        Ok(()) => panic!("stale checkpoint sequence was accepted"),
        Err(error) => error,
    };
    assert!(
        error.contains("checkpoint sequence changed"),
        "unexpected error: {error}"
    );
}
