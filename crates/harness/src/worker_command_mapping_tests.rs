// SPDX-License-Identifier: MIT

use crate::execution::{
    ExecutionFingerprint, ExecutionStore, WORKER_EMPTY_PARAMETERS_DIGEST, WorkerBoot,
    WorkerControlMode, WorkerControlRequest, WorkerOwnerProof,
};
use crate::worker_handoff::{
    SCHEMA_DIGEST, WorkerCapability, WorkerCommandAdmission, WorkerCommandConfig, WorkerRequest,
};
use serde_json::{Map, Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DEPLOYMENT_ID: &str = "deployment-1";
const WORKER_OWNER_ID: &str = "harness";
const WORKER_BOOT_ID: &str = "66666666-6666-4666-8666-666666666666";
const WATCHDOG_BOOT_ID: &str = "88888888-8888-4888-8888-888888888888";
const REQUEST_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const FRESH_HANDOFF_ID: &str = "99999999-9999-4999-8999-999999999999";
const FRESH_RUN_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const FRESH_EPISODE_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const FRESH_TRAJECTORY_ID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";

fn profile() -> String {
    "a".repeat(64)
}

fn fingerprint() -> Result<ExecutionFingerprint, Box<dyn std::error::Error>> {
    Ok(ExecutionFingerprint::new(
        "seed-1",
        "b".repeat(64),
        "d".repeat(64),
        "c".repeat(64),
        "e".repeat(64),
    )?)
}

fn admission() -> Result<WorkerCommandAdmission, Box<dyn std::error::Error>> {
    Ok(WorkerCommandAdmission::new(WorkerCommandConfig::new(
        DEPLOYMENT_ID,
        WORKER_OWNER_ID,
        profile(),
        "b".repeat(64),
        "c".repeat(64),
        WORKER_BOOT_ID,
        WATCHDOG_BOOT_ID,
    )?)?)
}

fn running_store() -> Result<ExecutionStore, Box<dyn std::error::Error>> {
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_worker_boot(&WorkerBoot::new(
        DEPLOYMENT_ID,
        WORKER_OWNER_ID,
        profile(),
        WORKER_BOOT_ID,
    )?)?;
    store.set_worker_control_mode(
        &WorkerControlRequest::new(
            DEPLOYMENT_ID,
            WORKER_OWNER_ID,
            profile(),
            WATCHDOG_BOOT_ID,
            WORKER_BOOT_ID,
            WorkerControlMode::Running,
            1,
        )?,
        &WorkerOwnerProof::new("transport-owner")?,
    )?;
    Ok(store)
}

fn authenticated(
    request: WorkerRequest,
) -> Result<crate::worker_handoff::AuthenticatedWorkerRequest, Box<dyn std::error::Error>> {
    Ok(
        crate::worker_handoff::AuthenticatedWorkerRequest::from_transport(
            request,
            WorkerCapability::Dispatch,
            WorkerOwnerProof::new("transport-owner")?,
        ),
    )
}

fn dispatch_request(
    handoff_id: &str,
    job_id: &str,
    attempt_id: &str,
    run_id: &str,
    episode_id: &str,
    trajectory_id: &str,
    worker_profile_digest: &str,
) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let fields = dispatch_fields(
        handoff_id,
        job_id,
        attempt_id,
        run_id,
        episode_id,
        trajectory_id,
        worker_profile_digest,
    );
    Ok(WorkerRequest::decode(&serde_json::to_vec(&fields)?)?)
}

fn dispatch_fields(
    handoff_id: &str,
    job_id: &str,
    attempt_id: &str,
    run_id: &str,
    episode_id: &str,
    trajectory_id: &str,
    worker_profile_digest: &str,
) -> Map<String, Value> {
    Map::from_iter([
        (
            "contract".into(),
            json!("ascension-watchdog-worker-handoff-v1"),
        ),
        ("schema_digest".into(), json!(SCHEMA_DIGEST)),
        ("direction".into(), json!("request")),
        ("command".into(), json!("dispatch")),
        ("scope".into(), json!("dispatch")),
        ("request_id".into(), json!(REQUEST_ID)),
        ("timeout_ms".into(), json!(1000)),
        ("watchdog_boot_id".into(), json!(WATCHDOG_BOOT_ID)),
        ("worker_boot_id".into(), json!(WORKER_BOOT_ID)),
        ("mode_sequence".into(), json!(1)),
        ("operation".into(), json!("runtime_v3_episode")),
        ("parameters".into(), json!({})),
        ("handoff_id".into(), json!(handoff_id)),
        ("deployment_id".into(), json!(DEPLOYMENT_ID)),
        ("job_id".into(), json!(job_id)),
        ("attempt_id".into(), json!(attempt_id)),
        ("attempt_number".into(), json!(1)),
        ("worker_owner_id".into(), json!(WORKER_OWNER_ID)),
        ("worker_profile_digest".into(), json!(worker_profile_digest)),
        ("run_id".into(), json!(run_id)),
        ("episode_id".into(), json!(episode_id)),
        ("trajectory_id".into(), json!(trajectory_id)),
        (
            "payload_digest".into(),
            json!(WORKER_EMPTY_PARAMETERS_DIGEST),
        ),
    ])
}

#[test]
fn authenticated_mapping_uses_fresh_tuple_ids_and_static_harness_policy() -> TestResult {
    let mut store = running_store()?;
    let handler = admission()?;
    let request = authenticated(dispatch_request(
        FRESH_HANDOFF_ID,
        "job-fresh",
        "attempt-fresh",
        FRESH_RUN_ID,
        FRESH_EPISODE_ID,
        FRESH_TRAJECTORY_ID,
        &profile(),
    )?)?;
    let preparation =
        handler.prepare_dispatch_from_authenticated(&mut store, &request, fingerprint()?)?;
    let (_, reservation) = handler
        .admit_prepared_dispatch(&mut store, &request, preparation)?
        .into_parts();
    let reservation = reservation.ok_or("missing fresh reservation")?;
    assert_eq!(reservation.tuple().handoff_id, FRESH_HANDOFF_ID);
    assert_eq!(reservation.tuple().job_id, "job-fresh");
    assert_eq!(reservation.tuple().attempt_id, "attempt-fresh");
    let episode = store.load_episode(FRESH_EPISODE_ID)?;
    assert_eq!(episode.lineage.run_id, FRESH_RUN_ID);
    assert_eq!(episode.lineage.trajectory_id, FRESH_TRAJECTORY_ID);
    assert_eq!(episode.fingerprint, fingerprint()?);
    assert_eq!(store.job("job-fresh")?.episode_id, FRESH_EPISODE_ID);
    Ok(())
}

#[test]
fn authenticated_mapping_rejects_profile_and_semantic_changes() -> TestResult {
    let mut store = running_store()?;
    let handler = admission()?;
    let mismatched_profile = authenticated(dispatch_request(
        FRESH_HANDOFF_ID,
        "job-profile-mismatch",
        "attempt-profile-mismatch",
        FRESH_RUN_ID,
        FRESH_EPISODE_ID,
        FRESH_TRAJECTORY_ID,
        &"f".repeat(64),
    )?)?;
    assert!(matches!(
        handler.prepare_dispatch_from_authenticated(
            &mut store,
            &mismatched_profile,
            fingerprint()?
        ),
        Err(crate::worker_handoff::WorkerCommandError::IdentityMismatch)
    ));

    let mut semantic_change = dispatch_fields(
        FRESH_HANDOFF_ID,
        "job-semantic-change",
        "attempt-semantic-change",
        FRESH_RUN_ID,
        FRESH_EPISODE_ID,
        FRESH_TRAJECTORY_ID,
        &profile(),
    );
    semantic_change.insert("parameters".into(), json!({"seed": "changed"}));
    assert!(WorkerRequest::decode(&serde_json::to_vec(&semantic_change)?).is_err());
    Ok(())
}
