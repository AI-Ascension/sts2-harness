// SPDX-License-Identifier: MIT

use crate::execution::{
    Checkpoint, CompletionRecord, CompletionStatus, ExecutionFingerprint, ExecutionLineage,
    ExecutionStore, WORKER_EMPTY_PARAMETERS_DIGEST, WorkerAdmissionContext, WorkerBoot,
    WorkerControlMode, WorkerControlRequest, WorkerHandoffState, WorkerOwnerProof, WorkerTuple,
};
use crate::worker_handoff::{
    AcknowledgmentStatus, DispatchReply, HandoffError, LookupReply, SCHEMA_DIGEST, WorkerReply,
    WorkerRequest,
};
use serde_json::{Map, Value, json};

use super::WorkerCommandAdmission;
use super::support::{
    ApprovedWorkerExecution, AuthenticatedWorkerRequest, WorkerCapability, WorkerCommandConfig,
    WorkerCommandError, WorkerDispatchPreparation,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

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

fn config() -> WorkerCommandConfig {
    WorkerCommandConfig {
        deployment_id: String::from("deployment-1"),
        worker_owner_id: String::from("harness"),
        worker_profile_digest: profile(),
        release_digest: "b".repeat(64),
        config_digest: "c".repeat(64),
        worker_boot_id: String::from(WORKER_BOOT_ID),
        watchdog_boot_id: String::from(WATCHDOG_BOOT_ID),
    }
}

fn fingerprint() -> Result<ExecutionFingerprint, WorkerCommandError> {
    ExecutionFingerprint::new(
        "seed-1",
        "b".repeat(64),
        "d".repeat(64),
        "c".repeat(64),
        "e".repeat(64),
    )
    .map_err(|_| WorkerCommandError::InvalidBinding)
}

fn lineage() -> Result<ExecutionLineage, WorkerCommandError> {
    ExecutionLineage::new(RUN_ID, EPISODE_ID, "attempt-1", TRAJECTORY_ID)
        .map_err(|_| WorkerCommandError::InvalidBinding)
}

fn admission() -> Result<WorkerCommandAdmission, WorkerCommandError> {
    WorkerCommandAdmission::new(config())
}

fn approved() -> Result<ApprovedWorkerExecution, WorkerCommandError> {
    ApprovedWorkerExecution::new(lineage()?, fingerprint()?, "job-1", 1)
}

fn running_store() -> Result<(ExecutionStore, WorkerAdmissionContext), Box<dyn std::error::Error>> {
    let mut store = ExecutionStore::open_in_memory()?;
    let boot = WorkerBoot::new("deployment-1", "harness", profile(), WORKER_BOOT_ID)?;
    store.start_worker_boot(&boot)?;
    let proof = WorkerOwnerProof::new("transport-owner")?;
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
    Ok((
        store,
        WorkerAdmissionContext::new(WATCHDOG_BOOT_ID, WORKER_BOOT_ID, 1)?,
    ))
}

fn tuple() -> Result<WorkerTuple, Box<dyn std::error::Error>> {
    Ok(WorkerTuple::new(
        HANDOFF_ID,
        "deployment-1",
        "job-1",
        "attempt-1",
        1,
        "harness",
        profile(),
        RUN_ID,
        EPISODE_ID,
        TRAJECTORY_ID,
        WORKER_EMPTY_PARAMETERS_DIGEST,
    )?)
}

fn admitted_store() -> Result<ExecutionStore, Box<dyn std::error::Error>> {
    let (mut store, context) = running_store()?;
    store.start_episode(&lineage()?, &fingerprint().map_err(|_| HandoffError)?)?;
    store.admit_job("job-1", EPISODE_ID, WORKER_EMPTY_PARAMETERS_DIGEST)?;
    store.admit_worker_handoff(&tuple()?, &context)?;
    Ok(store)
}

fn prepared_dispatch() -> Result<
    (
        ExecutionStore,
        WorkerCommandAdmission,
        AuthenticatedWorkerRequest,
        WorkerDispatchPreparation,
    ),
    Box<dyn std::error::Error>,
> {
    let (mut store, _) = running_store()?;
    let handler = admission()?;
    let request = authenticated(dispatch_request()?, WorkerCapability::Dispatch)?;
    let preparation = handler.prepare_dispatch(&mut store, &request, approved()?)?;
    Ok((store, handler, request, preparation))
}

fn fields(command: &str, scope: &str) -> Map<String, Value> {
    Map::from_iter([
        (
            "contract".into(),
            json!("ascension-watchdog-worker-handoff-v1"),
        ),
        ("schema_digest".into(), json!(SCHEMA_DIGEST)),
        ("direction".into(), json!("request")),
        ("command".into(), json!(command)),
        ("scope".into(), json!(scope)),
        ("request_id".into(), json!(REQUEST_ID)),
        ("timeout_ms".into(), json!(1000)),
        ("watchdog_boot_id".into(), json!(WATCHDOG_BOOT_ID)),
    ])
}

fn decode(fields: Map<String, Value>) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    Ok(WorkerRequest::decode(&serde_json::to_vec(&fields)?)?)
}

fn probe_request(watchdog: &str) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let mut fields = fields("probe", "probe");
    fields.insert("watchdog_boot_id".into(), json!(watchdog));
    decode(fields)
}

fn tuple_fields(fields: &mut Map<String, Value>) {
    fields.extend(Map::from_iter([
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
    ]));
}

fn dispatch_request() -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let mut fields = fields("dispatch", "dispatch");
    fields.insert("worker_boot_id".into(), json!(WORKER_BOOT_ID));
    fields.insert("mode_sequence".into(), json!(1));
    fields.insert("operation".into(), json!("runtime_v3_episode"));
    fields.insert("parameters".into(), json!({}));
    tuple_fields(&mut fields);
    decode(fields)
}

fn lookup_request_for(deployment: &str) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let mut fields = fields("lookup", "lookup");
    fields.insert("worker_boot_id".into(), json!(WORKER_BOOT_ID));
    tuple_fields(&mut fields);
    fields.insert("deployment_id".into(), json!(deployment));
    decode(fields)
}

fn acknowledge_request(digest: &str) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let mut fields = fields("acknowledge", "acknowledge");
    fields.insert("worker_boot_id".into(), json!(WORKER_BOOT_ID));
    fields.insert("terminal_digest".into(), json!(digest));
    tuple_fields(&mut fields);
    decode(fields)
}

fn control_request(mode: &str, sequence: u64) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    let mut fields = fields("set_control_mode", "control");
    fields.insert("worker_boot_id".into(), json!(WORKER_BOOT_ID));
    fields.extend(Map::from_iter([
        ("deployment_id".into(), json!("deployment-1")),
        ("worker_owner_id".into(), json!("harness")),
        ("worker_profile_digest".into(), json!(profile())),
        ("mode".into(), json!(mode)),
        ("mode_sequence".into(), json!(sequence)),
    ]));
    decode(fields)
}

fn authenticated(
    request: WorkerRequest,
    capability: WorkerCapability,
) -> Result<AuthenticatedWorkerRequest, Box<dyn std::error::Error>> {
    Ok(AuthenticatedWorkerRequest {
        request,
        capability,
        owner_proof: WorkerOwnerProof::new("transport-owner")?,
    })
}

#[test]
fn probe_readiness_tracks_durable_control() -> TestResult {
    let (mut store, _) = running_store()?;
    let handler = admission()?;
    let request = authenticated(probe_request(WATCHDOG_BOOT_ID)?, WorkerCapability::Probe)?;
    let result = handler.handle_authenticated(&mut store, &request)?;
    assert!(matches!(result.reply, WorkerReply::Probe(reply) if reply.ready));
    let mut stopped = ExecutionStore::open_in_memory()?;
    let stopped_result = handler.handle_authenticated(&mut stopped, &request)?;
    assert!(matches!(
        stopped_result.reply,
        WorkerReply::Probe(reply) if !reply.ready
    ));
    Ok(())
}

#[test]
fn capability_boot_and_tuple_binding_fail_closed() -> TestResult {
    let (mut store, _) = running_store()?;
    let handler = admission()?;
    let wrong_capability =
        authenticated(probe_request(WATCHDOG_BOOT_ID)?, WorkerCapability::Lookup)?;
    assert!(matches!(
        handler.handle_authenticated(&mut store, &wrong_capability),
        Err(WorkerCommandError::Unauthorized)
    ));
    let wrong_boot = authenticated(
        probe_request("99999999-9999-4999-8999-999999999999")?,
        WorkerCapability::Probe,
    )?;
    assert!(matches!(
        handler.handle_authenticated(&mut store, &wrong_boot),
        Err(WorkerCommandError::IdentityMismatch)
    ));
    let wrong_tuple = authenticated(
        lookup_request_for("other-deployment")?,
        WorkerCapability::Lookup,
    )?;
    assert!(matches!(
        handler.handle_authenticated(&mut store, &wrong_tuple),
        Err(WorkerCommandError::IdentityMismatch)
    ));
    Ok(())
}

#[test]
fn control_maps_only_authenticated_owner_update() -> TestResult {
    let (mut store, _) = running_store()?;
    let handler = admission()?;
    let request = authenticated(
        control_request("paused", 2)?,
        WorkerCapability::SetControlMode,
    )?;
    let result = handler.handle_authenticated(&mut store, &request)?;
    assert!(matches!(
        result.reply,
        WorkerReply::Control { accepted: true }
    ));
    assert_eq!(
        store.worker_control()?.map(|state| state.mode),
        Some(WorkerControlMode::Paused)
    );
    Ok(())
}

#[test]
fn historical_lookup_and_acknowledgment_never_create_a_reservation() -> TestResult {
    let mut store = admitted_store()?;
    let handler = admission()?;
    let lookup = authenticated(
        lookup_request_for("deployment-1")?,
        WorkerCapability::Lookup,
    )?;
    let result = handler.handle_authenticated(&mut store, &lookup)?;
    assert!(matches!(
        result.reply,
        WorkerReply::Lookup(LookupReply::Running)
    ));

    store.save_checkpoint(&Checkpoint::new(
        lineage()?,
        1,
        "state-1",
        1,
        fingerprint().map_err(|_| HandoffError)?,
        b"{}".to_vec(),
        "catalog-1",
    )?)?;
    store.record_completion(&CompletionRecord::new(
        lineage().map_err(|_| HandoffError)?,
        CompletionStatus::Completed,
        "terminal-1",
        1,
        "f".repeat(64),
    )?)?;
    let terminal = match handler.handle_authenticated(&mut store, &lookup)?.reply {
        WorkerReply::Lookup(LookupReply::Terminal(terminal)) => terminal,
        _ => return Err("lookup did not project durable terminal".into()),
    };
    let digest = terminal.acknowledgment_digest()?;
    let dispatch = authenticated(dispatch_request()?, WorkerCapability::Dispatch)?;
    let preparation = handler.prepare_dispatch(&mut store, &dispatch, approved()?)?;
    let mut duplicate = handler.admit_prepared_dispatch(&mut store, &dispatch, preparation)?;
    assert!(matches!(
        duplicate.reply,
        WorkerReply::Dispatch(DispatchReply::AlreadyCompleted(_))
    ));
    assert!(duplicate.take_reservation().is_none());
    let acknowledgment =
        authenticated(acknowledge_request(&digest)?, WorkerCapability::Acknowledge)?;
    let first = handler.handle_authenticated(&mut store, &acknowledgment)?;
    assert!(matches!(
        first.reply,
        WorkerReply::Acknowledge(AcknowledgmentStatus::Acknowledged)
    ));
    let second = handler.handle_authenticated(&mut store, &acknowledgment)?;
    assert!(matches!(
        second.reply,
        WorkerReply::Acknowledge(AcknowledgmentStatus::AlreadyAcknowledged)
    ));
    assert_eq!(
        store
            .worker_handoff(HANDOFF_ID)?
            .map(|handoff| handoff.state),
        Some(WorkerHandoffState::Acknowledged)
    );
    Ok(())
}

#[test]
fn dispatch_requires_approved_preparation_and_atomic_store_outcome() -> TestResult {
    let (mut store, _) = running_store()?;
    let handler = admission()?;
    let request = authenticated(dispatch_request()?, WorkerCapability::Dispatch)?;
    assert!(matches!(
        handler.handle_authenticated(&mut store, &request),
        Err(WorkerCommandError::ReservationMismatch)
    ));

    let preparation = handler.prepare_dispatch(&mut store, &request, approved()?)?;
    let second_preparation = handler.prepare_dispatch(&mut store, &request, approved()?)?;

    let mut winner = handler.admit_prepared_dispatch(&mut store, &request, preparation)?;
    assert!(matches!(
        winner.reply,
        WorkerReply::Dispatch(DispatchReply::Accepted)
    ));
    let reservation = winner.take_reservation().ok_or("missing permit")?;
    assert_eq!(reservation.tuple().handoff_id, HANDOFF_ID);
    let running = reservation.start(&mut store)?;
    assert_eq!(running.state, WorkerHandoffState::Running);

    let mut duplicate =
        handler.admit_prepared_dispatch(&mut store, &request, second_preparation)?;
    assert!(matches!(
        duplicate.reply,
        WorkerReply::Dispatch(DispatchReply::Busy)
    ));
    assert!(duplicate.take_reservation().is_none());
    Ok(())
}

#[test]
fn reservation_is_fenced_when_control_changes_before_consumption() -> TestResult {
    for mode in ["paused", "running"] {
        let (mut store, handler, request, preparation) = prepared_dispatch()?;
        let mut result = handler.admit_prepared_dispatch(&mut store, &request, preparation)?;
        let reservation = result.take_reservation().ok_or("missing permit")?;
        let control = authenticated(control_request(mode, 2)?, WorkerCapability::SetControlMode)?;
        assert!(matches!(
            handler.handle_authenticated(&mut store, &control)?.reply,
            WorkerReply::Control { accepted: true }
        ));
        assert!(matches!(
            reservation.start(&mut store),
            Err(WorkerCommandError::ReservationMismatch)
        ));
    }
    Ok(())
}

#[test]
fn preparation_rejects_missing_control_or_wrong_approved_fingerprint() -> TestResult {
    let mut store = ExecutionStore::open_in_memory()?;
    let handler = admission()?;
    let request = authenticated(dispatch_request()?, WorkerCapability::Dispatch)?;
    assert!(matches!(
        handler.prepare_dispatch(&mut store, &request, approved()?),
        Err(WorkerCommandError::ReservationMismatch)
    ));
    let (mut store, _) = running_store()?;
    let wrong_fingerprint = ExecutionFingerprint::new(
        "seed-1",
        "d".repeat(64),
        "d".repeat(64),
        "c".repeat(64),
        "e".repeat(64),
    )?;
    let approved = ApprovedWorkerExecution::new(lineage()?, wrong_fingerprint, "job-1", 1)?;
    assert!(matches!(
        handler.prepare_dispatch(&mut store, &request, approved),
        Err(WorkerCommandError::InvalidBinding)
    ));
    Ok(())
}
