// SPDX-License-Identifier: MIT

use super::super::worker_store::{
    begin_quarantine, finish_quarantine, try_lock, try_lock_recovery,
};
use super::*;
use serde_json::{Map, json};
use sts2_harness::worker_handoff::{SCHEMA_DIGEST, WorkerCapability, WorkerRequest};
use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, WORKER_EMPTY_PARAMETERS_DIGEST, WorkerBoot,
    WorkerControlMode, WorkerControlRequest, WorkerOwnerProof,
};

const RUN_ID: &str = "11111111-1111-4111-8111-111111111111";
const EPISODE_ID: &str = "22222222-2222-4222-8222-222222222222";
const TRAJECTORY_ID: &str = "33333333-3333-4333-8333-333333333333";
const HANDOFF_ID: &str = "44444444-4444-4444-8444-444444444444";
const WORKER_BOOT_ID: &str = "66666666-6666-4666-8666-666666666666";
const WATCHDOG_BOOT_ID: &str = "88888888-8888-4888-8888-888888888888";
const REQUEST_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

fn profile() -> String {
    "a".repeat(64)
}

fn lineage() -> Result<ExecutionLineage, Box<dyn std::error::Error>> {
    Ok(ExecutionLineage::new(
        RUN_ID,
        EPISODE_ID,
        "attempt-1",
        TRAJECTORY_ID,
    )?)
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

fn request() -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let fields = Map::from_iter([
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
        ("handoff_id".into(), json!(HANDOFF_ID)),
        ("deployment_id".into(), json!("deployment-1")),
        ("job_id".into(), json!("job-1")),
        ("attempt_id".into(), json!("attempt-1")),
        ("attempt_number".into(), json!(1)),
        ("worker_owner_id".into(), json!("harness")),
        ("worker_profile_digest".into(), json!(profile())),
        ("run_id".into(), json!(RUN_ID)),
        ("episode_id".into(), json!(EPISODE_ID)),
        ("trajectory_id".into(), json!(TRAJECTORY_ID)),
        (
            "payload_digest".into(),
            json!(WORKER_EMPTY_PARAMETERS_DIGEST),
        ),
    ]);
    Ok(WorkerRequest::decode(&serde_json::to_vec(&fields)?)?)
}

fn runtime() -> Result<WorkerRuntime, Box<dyn std::error::Error>> {
    let mut store = ExecutionStore::open_in_memory()?;
    let boot = WorkerBoot::new("deployment-1", "harness", profile(), WORKER_BOOT_ID)?;
    store.start_worker_boot(&boot)?;
    let proof = WorkerOwnerProof::new("test-owner")?;
    let control = WorkerControlRequest::new(
        "deployment-1",
        "harness",
        profile(),
        WATCHDOG_BOOT_ID,
        WORKER_BOOT_ID,
        WorkerControlMode::Running,
        1,
    )?;
    store.set_worker_control_mode(&control, &proof)?;
    store.start_episode(&lineage()?, &fingerprint()?)?;
    store.admit_job("job-1", EPISODE_ID, WORKER_EMPTY_PARAMETERS_DIGEST)?;
    let command = WorkerCommandConfig::new(
        "deployment-1",
        "harness",
        profile(),
        "b".repeat(64),
        "c".repeat(64),
        WORKER_BOOT_ID,
        WATCHDOG_BOOT_ID,
    )?;
    Ok(WorkerRuntime::from_shared_store(
        share_store(store),
        command,
        fingerprint()?,
        String::from(WORKER_BOOT_ID),
    )?)
}

fn authenticated(
    request: WorkerRequest,
) -> Result<AuthenticatedWorkerRequest, Box<dyn std::error::Error>> {
    Ok(AuthenticatedWorkerRequest::from_transport(
        request,
        WorkerCapability::Dispatch,
        WorkerOwnerProof::new("test-owner")?,
    ))
}

fn admitted_reservation(
    runtime: &mut WorkerRuntime,
) -> Result<WorkerExecutionReservation, Box<dyn std::error::Error>> {
    let exchange = runtime.handle_authenticated(&authenticated(request()?)?)?;
    let (_, reservation) = exchange.into_parts();
    reservation.ok_or_else(|| "dispatch did not return an execution reservation".into())
}

fn handoff_state(
    runtime: &WorkerRuntime,
) -> Result<WorkerHandoffState, Box<dyn std::error::Error>> {
    let store = try_lock_recovery(&runtime.store)?;
    Ok(store
        .worker_handoff(HANDOFF_ID)?
        .ok_or("missing handoff")?
        .state)
}

#[test]
fn failed_response_retains_unknown_and_never_starts() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    let authenticated = authenticated(request()?)?;
    let exchange = runtime.handle_authenticated(&authenticated)?;
    let outcome = runtime.finish_exchange(exchange, ResponseWriteStatus::Failed)?;
    assert!(matches!(outcome, WorkerStartOutcome::Unknown { .. }));
    let store = try_lock_recovery(&runtime.store)?;
    let handoff = store.worker_handoff(HANDOFF_ID)?.ok_or("missing handoff")?;
    assert_eq!(handoff.state, WorkerHandoffState::Unknown);
    Ok(())
}

#[test]
fn successful_response_crosses_the_running_fence() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    let request = request()?;
    let authenticated = authenticated(request.clone())?;
    let exchange = runtime.handle_authenticated(&authenticated)?;
    let (reply, reservation) = exchange.into_parts();
    let response = request.encode_response(WORKER_BOOT_ID, reply)?;
    assert!(!response.is_empty());
    let outcome = runtime.finish_reservation(reservation, ResponseWriteStatus::Written)?;
    assert!(matches!(outcome, WorkerStartOutcome::Started(_)));
    let store = try_lock(&runtime.store)?;
    let handoff = store.worker_handoff(HANDOFF_ID)?.ok_or("missing handoff")?;
    assert_eq!(handoff.state, WorkerHandoffState::Running);
    Ok(())
}

#[test]
fn held_store_lock_retains_primary_and_quarantine_diagnostics()
-> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    let reservation = admitted_reservation(&mut runtime)?;
    let shared_store = runtime.store.clone();
    let held_store = try_lock_recovery(&shared_store)?;
    let error = match runtime.finish_reservation(Some(reservation), ResponseWriteStatus::Written) {
        Ok(_) => return Err("held store lock unexpectedly started a reservation".into()),
        Err(error) => error,
    };
    assert!(
        error.contains("execution store is busy"),
        "primary: {error}"
    );
    assert!(
        error.contains("failed to retain unknown worker handoff"),
        "quarantine: {error}"
    );
    assert_eq!(runtime.lane.active_handoff_id(), Some(HANDOFF_ID));
    drop(held_store);
    assert_eq!(handoff_state(&runtime)?, WorkerHandoffState::Admitted);

    let retry = runtime.handle_authenticated(&authenticated(request()?)?);
    let retry_error = match retry {
        Ok(_) => return Err("quarantine-pending gate admitted a fresh dispatch".into()),
        Err(error) => error,
    };
    assert!(retry_error.contains("fail-closed"), "retry: {retry_error}");
    assert_eq!(handoff_state(&runtime)?, WorkerHandoffState::Admitted);
    Ok(())
}

#[test]
fn already_quarantined_gate_accounts_for_admitted_reservation()
-> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    let reservation = admitted_reservation(&mut runtime)?;
    assert!(!begin_quarantine(&runtime.store)?);
    finish_quarantine(&runtime.store);

    let outcome = runtime.finish_reservation(Some(reservation), ResponseWriteStatus::Written)?;
    assert!(matches!(outcome, WorkerStartOutcome::Unknown { .. }));
    assert_eq!(handoff_state(&runtime)?, WorkerHandoffState::Unknown);
    assert_eq!(runtime.lane.active_handoff_id(), Some(HANDOFF_ID));

    let retry = runtime.handle_authenticated(&authenticated(request()?)?);
    let retry_error = match retry {
        Ok(_) => return Err("quarantined gate admitted a fresh dispatch".into()),
        Err(error) => error,
    };
    assert!(retry_error.contains("fail-closed"), "retry: {retry_error}");
    assert_eq!(handoff_state(&runtime)?, WorkerHandoffState::Unknown);
    Ok(())
}

#[test]
fn closed_store_preserves_start_and_unknown_retention_failures()
-> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = runtime()?;
    let reservation = admitted_reservation(&mut runtime)?;
    {
        let mut store = try_lock_recovery(&runtime.store)?;
        store.close()?;
    }

    let error = match runtime.finish_reservation(Some(reservation), ResponseWriteStatus::Written) {
        Ok(_) => return Err("closed store unexpectedly started a reservation".into()),
        Err(error) => error,
    };
    assert!(
        error.contains("worker command durable store operation failed"),
        "primary: {error}"
    );
    assert!(
        error.contains("cannot retain uncertain worker handoff"),
        "quarantine: {error}"
    );
    assert_eq!(runtime.lane.active_handoff_id(), Some(HANDOFF_ID));

    let retry = runtime.handle_authenticated(&authenticated(request()?)?);
    let retry_error = match retry {
        Ok(_) => return Err("closed-store gate admitted a fresh dispatch".into()),
        Err(error) => error,
    };
    assert!(retry_error.contains("fail-closed"), "retry: {retry_error}");
    Ok(())
}

#[test]
fn missing_transport_is_a_fixed_production_failure() {
    assert_eq!(
        MISSING_TRANSPORT_ERROR,
        "worker transport unavailable: authenticated native worker listener is not integrated"
    );
}
