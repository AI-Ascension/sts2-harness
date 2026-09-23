// SPDX-License-Identifier: MIT

//! `#94` live submission fences that must refuse before the first effect.
//!
//! A repeated live run identity must be refused *before* the submission is reserved, a session is
//! opened or an episode is launched, and a submission whose carried definition does not hash to the
//! digest its admission is bound to must be refused at the same point. The unit under test is
//! [`LiveWorkflowExecutionPort::submit_admitted_with_reservation`]; the composition is the real
//! production session and only the gateway/MCP runtime and the provider are fixtures.
//!
//! The reservation store is a counting double whose `create_run` accepts a repeated
//! `(request_id, request_digest)` pair. That models the divergence `#94` names — the in-process
//! registry still holds the run while the durable submission record is gone — and it is what makes
//! the ordering observable: a store that refused the duplicate itself would mask the port's
//! ordering behind its own error.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use super::{Counters, Policy, Provider, RuntimeFactory, Shared};
use crate::episode::{EpisodeObservation, EpisodeStage, RuntimeLeaseBinding};
use crate::management::store::SubmissionLookup;
use crate::management::{
    AuthContext, CommandAcceptance, CommandRequest, CommandResponse, EventPage, ExecutionMode,
    LiveTargetCatalogPort, LiveWorkflowExecutionPort, LiveWorkflowOptions,
    LiveWorkflowSessionFactory, MANAGEMENT_SCHEMA_VERSION, ManagementError,
    ProductionLiveWorkflowSessionFactory, RunAdmission, RunEvent, RunRequest, RunReservation,
    RunSnapshot, RunTargetConfiguration, RuntimeAuthorityBinding, StoreError,
    TARGET_ADMISSION_SCHEMA_VERSION, TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionBinding,
    TargetAvailability, TargetCatalogResponse, TargetDescriptor, WorkflowExecutionPort,
    WorkflowRunStatus, WorkflowStore, digest_value, live_run_id,
};
use crate::provider_session::NativeCapabilities;

const REQUEST_ID: &str = "duplicate-run-fence";
const INSTANCE_ID: &str = "instance-1";
const PROFILE: &str = "live.workflow.v1";
const GAME_PROFILE: &str = "sts2-live-v1";
const CATALOG_REVISION: &str = "live.catalog.v1";
const COMPATIBILITY_REVISION: &str = "live.compatibility.v1";
const CAPABILITY_REVISION: &str = "live.capabilities.v1";
const SESSION_ID: &str = "runtime-session";

/// The `valid-strict` vector re-shaped for live admission; the same seam the served-live
/// integration fixtures use, restated here because those support modules belong to a separate
/// crate.
fn live_definition() -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition fixture");
    value["annotations"]["synthetic"] = serde_json::json!(false);
    value["game_profile"] = serde_json::json!(GAME_PROFILE);
    value["policy_ref"] = serde_json::json!("policy.live.v1");
    value["capabilities"]["required"][0] = serde_json::json!("observe.fair-play.live.v1");
    value
}

pub(super) fn target_descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: INSTANCE_ID.to_owned(),
        execution_profiles: vec![PROFILE.to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: COMPATIBILITY_REVISION.to_owned(),
        capability_revision: CAPABILITY_REVISION.to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: vec!["observe.fair-play.live.v1".to_owned()],
        game_profiles: vec![GAME_PROFILE.to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: Vec::new(),
    }
}

/// Authoritative discovery double that serves the exact catalog the admitted binding names.
struct LiveCatalog;

impl LiveTargetCatalogPort for LiveCatalog {
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(TargetCatalogResponse {
            schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
            catalog_revision: CATALOG_REVISION.to_owned(),
            targets: vec![target_descriptor()],
        })
    }
}

/// The reservation store double. `create_run` records each reservation instead of rejecting the
/// repeated key, so the port — not the store — decides whether the duplicate is refused.
pub(super) struct CountingStore {
    creates: Mutex<Vec<(String, String)>>,
}

impl CountingStore {
    pub(super) fn new() -> Self {
        Self {
            creates: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn creates(&self) -> usize {
        self.creates.lock().expect("store lock").len()
    }
}

impl WorkflowStore for CountingStore {
    fn lookup_submission(
        &self,
        _request_id: &str,
        _request_digest: &str,
    ) -> Result<SubmissionLookup, StoreError> {
        Ok(SubmissionLookup::Missing)
    }

    fn create_run(
        &self,
        request_id: &str,
        request_digest: &str,
        _snapshot: RunSnapshot,
        _initial_events: Vec<RunEvent>,
    ) -> Result<(), StoreError> {
        self.creates
            .lock()
            .expect("store lock")
            .push((request_id.to_owned(), request_digest.to_owned()));
        Ok(())
    }

    fn get_run(&self, _run_id: &str) -> Result<Option<RunSnapshot>, StoreError> {
        Ok(None)
    }

    fn events(
        &self,
        _run_id: &str,
        _after_sequence: u64,
        _limit: u64,
    ) -> Result<EventPage, StoreError> {
        Err(unused("events"))
    }

    fn accept_command(
        &self,
        _request: &CommandRequest,
        _request_digest: &str,
    ) -> Result<CommandAcceptance, StoreError> {
        Err(unused("commands"))
    }

    fn apply_command(
        &self,
        _request: &CommandRequest,
        _request_digest: &str,
        _application: crate::management::store::CommandApplication,
    ) -> Result<CommandResponse, StoreError> {
        Err(unused("commands"))
    }

    fn release_command(
        &self,
        _request: &CommandRequest,
        _request_digest: &str,
    ) -> Result<(), StoreError> {
        Err(unused("commands"))
    }

    fn export(
        &self,
        _run_id: &str,
        _redacted: bool,
    ) -> Result<crate::management::ExportResponse, StoreError> {
        Err(unused("export"))
    }
}

pub(super) fn unused(surface: &str) -> StoreError {
    StoreError::new(
        "test_store_unused",
        format!("{surface} are not part of the duplicate-run fence"),
    )
}

pub(super) fn observation(state_id: &str, generation: u64) -> EpisodeObservation {
    EpisodeObservation::new(
        state_id,
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "fixture",
            "player": {"hp":1,"max_hp":1,"energy":1,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state": {"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions": [{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("observation")
}

/// One live request whose admission names the served catalog exactly, with its definition digest.
pub(super) fn admitted_request() -> (RunRequest, String) {
    let definition = live_definition();
    let definition_digest = digest_value(&definition).expect("definition digest");
    let binding = TargetAdmissionBinding {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: REQUEST_ID.to_owned(),
        workflow_definition_digest: definition_digest.clone(),
        target: RunTargetConfiguration {
            instance_id: INSTANCE_ID.to_owned(),
            execution_profile: PROFILE.to_owned(),
            execution_mode: ExecutionMode::Live,
            workflow_revision: "0.1.0".to_owned(),
            compatibility_revision: COMPATIBILITY_REVISION.to_owned(),
            capability_revision: CAPABILITY_REVISION.to_owned(),
            game_profile: GAME_PROFILE.to_owned(),
            save_profile: None,
            inference_profile: None,
            context_capability: None,
            provider_capability: None,
        },
        descriptor_digest: target_descriptor().digest().expect("descriptor digest"),
        catalog_revision: CATALOG_REVISION.to_owned(),
    };
    (
        RunRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            request_id: REQUEST_ID.to_owned(),
            definition: Some(definition),
            artifact_id: None,
            instance_id: INSTANCE_ID.to_owned(),
            profile: PROFILE.to_owned(),
            admission: Some(binding),
        },
        definition_digest,
    )
}

pub(super) fn counts(counters: &Shared<Counters>) -> (usize, usize, usize, usize) {
    let counters = counters.lock().expect("counter lock");
    (
        counters.runtime_opens,
        counters.dispatch_calls,
        counters.decide_calls,
        counters.provider_opens,
    )
}

pub(super) fn submit(
    port: &LiveWorkflowExecutionPort,
    request: &RunRequest,
    actor: &AuthContext,
    definition_digest: &str,
    request_digest: &str,
    store: Arc<dyn WorkflowStore>,
) -> Result<RunAdmission, ManagementError> {
    let reservation = RunReservation::new(
        store,
        request.request_id.clone(),
        request_digest.to_owned(),
        definition_digest.to_owned(),
        request.admission.clone(),
        None,
    );
    port.submit_admitted_with_reservation(
        request,
        actor,
        definition_digest,
        request.admission.as_ref(),
        &reservation,
    )
}

/// The real served composition, with the gateway/MCP runtime and the provider as fixtures.
///
/// `run_id` is the identity the fixture authority binding names. The port derives its own identity
/// from the request and the bound definition digest, so a submission that never reaches that
/// derivation still has to name the identity its configured authority would have carried.
pub(super) fn live_port(counters: &Shared<Counters>, run_id: &str) -> LiveWorkflowExecutionPort {
    let factory: Arc<dyn LiveWorkflowSessionFactory> = Arc::new(
        ProductionLiveWorkflowSessionFactory::new(
            serde_json::json!({"capabilities": []}),
            Arc::new(LiveCatalog),
            Arc::new(RuntimeFactory {
                authority: RuntimeAuthorityBinding {
                    instance_id: INSTANCE_ID.to_owned(),
                    session_id: SESSION_ID.to_owned(),
                    lease_id: "configured-lease".to_owned(),
                    lease_epoch: 1,
                    run_id: run_id.to_owned(),
                    episode_id: "duplicate-fence-episode".to_owned(),
                    trajectory_id: "duplicate-fence-trajectory".to_owned(),
                    trace_id: "duplicate-fence-trace".to_owned(),
                    artifact_id: "duplicate-fence-artifact".to_owned(),
                    agent_id: "duplicate-fence-agent".to_owned(),
                    adapter_revision: "duplicate-fence-adapter".to_owned(),
                    model_revision: "duplicate-fence-model".to_owned(),
                    configuration_digest: "b".repeat(64),
                    output_schema_digest: "c".repeat(64),
                },
                acquired: RuntimeLeaseBinding {
                    instance_id: INSTANCE_ID.to_owned(),
                    session_id: SESSION_ID.to_owned(),
                    run_id: run_id.to_owned(),
                    lease_id: "gateway-recovery-lease".to_owned(),
                    lease_epoch: 3,
                },
                observation: observation("combat-1", 1),
                counters: Arc::clone(counters),
            }),
            Arc::new(Provider {
                counters: Arc::clone(counters),
            }),
            Arc::new(Policy),
            NativeCapabilities::fixture(),
        )
        .expect("production factory"),
    );
    LiveWorkflowExecutionPort::new(factory, LiveWorkflowOptions::default()).expect("live port")
}

#[test]
fn live_duplicate_run_identity_is_refused_before_the_first_effect() {
    let (request, definition_digest) = admitted_request();
    let request_digest = digest_value(&serde_json::to_value(&request).expect("request encode"))
        .expect("request digest");
    let run_id = live_run_id(&request, &definition_digest).expect("run identity");
    let actor =
        AuthContext::new("duplicate-fence-actor", ["workflow:*".to_owned()]).expect("actor");

    let counters = Arc::new(Mutex::new(Counters::default()));
    let port = live_port(&counters, &run_id);
    let store = Arc::new(CountingStore::new());
    let reservation_store: Arc<dyn WorkflowStore> = store.clone();

    let admitted = submit(
        &port,
        &request,
        &actor,
        &definition_digest,
        &request_digest,
        Arc::clone(&reservation_store),
    )
    .expect("the first submission of an unused live identity must be admitted");
    assert_eq!(admitted.snapshot.workflow_run_id, run_id);
    assert_eq!(admitted.snapshot.status, WorkflowRunStatus::Running);
    assert_eq!(
        counts(&counters),
        (1, 0, 0, 1),
        "one admitted run opens exactly one runtime and one provider session"
    );

    // The registry still holds the run while the durable submission record is gone: the
    // lost-response/restart divergence that reaches the port with a repeated identity.
    let error = submit(
        &port,
        &request,
        &actor,
        &definition_digest,
        &request_digest,
        Arc::clone(&reservation_store),
    )
    .expect_err("a repeated live run identity must be refused");
    assert_eq!(error.code, "live_duplicate_run");
    assert_eq!(
        counts(&counters),
        (1, 0, 0, 1),
        "the refusal must not open a second runtime or a second provider session"
    );
    assert_eq!(
        store.creates(),
        1,
        "the refusal must not reserve the durable submission a second time"
    );
}

#[test]
fn live_definition_digest_mismatch_is_refused_before_the_first_effect() {
    // A request whose carried definition does not hash to the digest its admission is bound to is
    // the wrong-binding refusal `#94` AC2 names: the admission can approve one definition while the
    // submission ships another. The binding and the argument agree, so every admission fence passes
    // and only the byte-level digest comparison can catch it.
    let (mut request, _) = admitted_request();
    let mismatched = "f".repeat(64);
    request
        .admission
        .as_mut()
        .expect("admitted request carries an admission")
        .workflow_definition_digest = mismatched.clone();
    let request_digest = digest_value(&serde_json::to_value(&request).expect("request encode"))
        .expect("request digest");
    let run_id = live_run_id(&request, &mismatched).expect("run identity");
    let actor = AuthContext::new("identity-fence-actor", ["workflow:*".to_owned()]).expect("actor");

    let counters = Arc::new(Mutex::new(Counters::default()));
    let port = live_port(&counters, &run_id);
    let store = Arc::new(CountingStore::new());
    let reservation_store: Arc<dyn WorkflowStore> = store.clone();

    let error = submit(
        &port,
        &request,
        &actor,
        &mismatched,
        &request_digest,
        Arc::clone(&reservation_store),
    )
    .expect_err("a definition that does not match its bound digest must be refused");

    assert_eq!(error.code, "live_identity_mismatch");
    assert_eq!(
        counts(&counters),
        (0, 0, 0, 0),
        "the refusal must not open a runtime or reach a provider"
    );
    assert_eq!(
        store.creates(),
        0,
        "the refusal must not reserve a durable submission"
    );
}
