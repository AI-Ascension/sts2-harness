// SPDX-License-Identifier: MIT

//! Capability, definition, execution and target doubles for the inference
//! profile catalog admission suite.
//!
//! These doubles are synthetic and reach no provider, model, credential or
//! native host; the execution port only records that it was reached and mirrors
//! the production durable reservation so the service's own fences still run.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, Budget, CapabilityPort, CleanupState, CommandApplication, CommandContext, Cursor,
    DefinitionPort, Diagnostic, EVENT_SCHEMA_VERSION, EventClassification, EventPayload, EventType,
    ExecutionMode, GameOutcome, InferenceProfileCatalog, LiveInferenceProfileCatalogPort,
    MANAGEMENT_SCHEMA_VERSION, ManagementError, ManagementService, MemoryWorkflowStore,
    RunAdmission, RunEvent, RunRequest, RunSnapshot, RunTargetConfiguration,
    TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionBinding, TargetAvailability,
    TargetCatalogResponse, TargetDescriptor, ValidationResult, WorkflowExecutionPort,
    WorkflowRunStatus, WorkflowStore, digest_value,
};

use super::context_owner_double::FakeContextOwner;
use super::inference_profile_catalog_fixtures::*;

/// Serves a scripted catalog sequence; each call advances one step so a test
/// can observe that a refresh was read rather than cached.
pub(crate) struct CatalogCapabilityDouble {
    calls: Arc<AtomicUsize>,
    catalogs: Vec<InferenceProfileCatalog>,
    advertised: Vec<String>,
}

impl CatalogCapabilityDouble {
    pub(crate) fn serving(catalogs: Vec<InferenceProfileCatalog>) -> Arc<Self> {
        Self::advertising(catalogs, Vec::new())
    }

    /// Serves `catalogs` from a target whose descriptor advertises
    /// `advertised` inference profiles, so a test can submit a target-level
    /// selection the target list actually admits.
    pub(crate) fn advertising(
        catalogs: Vec<InferenceProfileCatalog>,
        advertised: Vec<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: Arc::new(AtomicUsize::new(0)),
            catalogs,
            advertised,
        })
    }

    fn selected(&self) -> InferenceProfileCatalog {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        self.catalogs[index.min(self.catalogs.len() - 1)].clone()
    }
}

impl CapabilityPort for CatalogCapabilityDouble {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({"capabilities": ["workflow.live", "live.workflow.v1"]}))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(target_catalog_advertising(&self.advertised))
    }

    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<Option<InferenceProfileCatalog>, ManagementError> {
        Ok(Some(self.selected()))
    }
}

/// A capability double whose catalog reads delegate to a mutable owner port, so
/// a test can replace the served revision between two service calls.
pub(crate) struct MutatingCatalogDouble {
    pub(crate) port: Arc<dyn LiveInferenceProfileCatalogPort>,
}

impl CapabilityPort for MutatingCatalogDouble {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({"capabilities": ["workflow.live", "live.workflow.v1"]}))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(target_catalog())
    }

    fn inference_profile_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<Option<InferenceProfileCatalog>, ManagementError> {
        self.port.inference_profile_catalog(actor).map(Some)
    }
}

pub(crate) struct MutableCatalogPort {
    pub(crate) catalog: std::sync::Mutex<InferenceProfileCatalog>,
}

impl LiveInferenceProfileCatalogPort for MutableCatalogPort {
    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<InferenceProfileCatalog, ManagementError> {
        self.catalog
            .lock()
            .map(|catalog| catalog.clone())
            .map_err(|_| ManagementError::store("catalog_lock", "catalog lock is poisoned"))
    }
}

pub(crate) struct DefinitionDouble;

impl DefinitionPort for DefinitionDouble {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Ok(ValidationResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            compiler: "test-compiler.v1".to_owned(),
            diagnostics: Vec::<Diagnostic>::new(),
        })
    }

    fn inspect(
        &self,
        _definition: &Value,
    ) -> Result<sts2_harness::management::InspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "inspection is not used by this test",
        ))
    }

    fn diff(
        &self,
        _old_definition: &Value,
        _new_definition: &Value,
    ) -> Result<sts2_harness::management::DiffResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "diff is not used by this test",
        ))
    }
}

/// Records how many times an execution port was reached and performs the same
/// durable reservation the live port performs, keyed by the request digest the
/// service computed before it rewrote the admission.
pub(crate) struct RecordingExecutionPort {
    submissions: Arc<AtomicUsize>,
    store: Arc<dyn WorkflowStore>,
    request_digest: String,
}

impl RecordingExecutionPort {
    fn new(store: Arc<dyn WorkflowStore>, request_digest: String) -> (Arc<Self>, Arc<AtomicUsize>) {
        let submissions = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                submissions: Arc::clone(&submissions),
                store,
                request_digest,
            }),
            submissions,
        )
    }

    fn reserve(
        &self,
        request: &RunRequest,
        definition_digest: &str,
    ) -> Result<(), ManagementError> {
        // The durable reservation must carry the same run identity the service
        // settles with, or the later snapshot update is rejected as an identity
        // conflict. Mirror the production reservation path exactly: the real
        // definition digest plus the already-bound provenance admission.
        let mut admitted = admission(definition_digest);
        admitted.snapshot.admission = request.admission.clone();
        self.store
            .create_run(
                &request.request_id,
                &self.request_digest,
                admitted.snapshot,
                admitted.initial_events,
            )
            .map_err(ManagementError::from)
    }
}

impl WorkflowExecutionPort for RecordingExecutionPort {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        Ok(admission(definition_digest))
    }

    fn submit_admitted_with_reservation(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        definition_digest: &str,
        _admission: Option<&TargetAdmissionBinding>,
        _reserve: &sts2_harness::management::RunReservation,
    ) -> Result<RunAdmission, ManagementError> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        self.reserve(request, definition_digest)?;
        let mut admitted = admission(definition_digest);
        admitted.snapshot.admission = request.admission.clone();
        Ok(admitted)
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "commands are not used by this test",
        ))
    }
}

pub(crate) fn admission(definition_digest: &str) -> RunAdmission {
    let snapshot = RunSnapshot {
        schema_version: "ascension.workflow-run/v1".to_owned(),
        workflow_run_id: "run-inference-fixture".to_owned(),
        definition_digest: definition_digest.to_owned(),
        run_revision: 1,
        status: WorkflowRunStatus::Created,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: "main".to_owned(),
            node_id: "decide".to_owned(),
            node_execution_id: "decide-exec".to_owned(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
        admission: None,
        execution_mode: None,
    };
    RunAdmission {
        initial_events: vec![RunEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_owned(),
            workflow_run_id: snapshot.workflow_run_id.clone(),
            sequence: 1,
            run_revision: 1,
            event_type: EventType::RunStarted,
            definition_digest: definition_digest.to_owned(),
            node_execution_id: "management".to_owned(),
            payload: EventPayload {
                operation_id: None,
                classification: Some(EventClassification::Accepted),
                reason_code: "submitted".to_owned(),
            },
            integrity_digest: None,
        }],
        snapshot,
    }
}

/// The single live fake target every admission in this suite is scoped to. It
/// names no real instance and reaches no host, and it advertises
/// `inference_profiles` the way the served runtime advertises its one reviewed
/// provider-session adapter.
pub(crate) fn target_descriptor_advertising(inference_profiles: Vec<String>) -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "instance-1".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: vec!["workflow.live".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles,
    }
}

pub(crate) fn target_catalog() -> TargetCatalogResponse {
    target_catalog_advertising(&[])
}

pub(crate) fn target_catalog_advertising(inference_profiles: &[String]) -> TargetCatalogResponse {
    TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: CATALOG_REVISION.to_owned(),
        targets: vec![target_descriptor_advertising(inference_profiles.to_vec())],
    }
}

/// The same target, carrying the consumer's target-level adapter selection.
pub(crate) fn target_selecting(selection: Option<&str>) -> RunTargetConfiguration {
    RunTargetConfiguration {
        instance_id: "instance-1".to_owned(),
        execution_profile: "live.workflow.v1".to_owned(),
        execution_mode: ExecutionMode::Live,
        workflow_revision: "0.1.0".to_owned(),
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        game_profile: "sts2-live-v1".to_owned(),
        save_profile: None,
        inference_profile: selection.map(str::to_owned),
        context_capability: None,
        provider_capability: None,
    }
}

pub(crate) fn binding(definition: &Value, request_id: &str) -> TargetAdmissionBinding {
    binding_selecting(definition, request_id, None)
}

pub(crate) fn binding_selecting(
    definition: &Value,
    request_id: &str,
    selection: Option<&str>,
) -> TargetAdmissionBinding {
    TargetAdmissionBinding {
        schema_version: "ascension.workflow-admission/v1".to_owned(),
        request_id: request_id.to_owned(),
        workflow_definition_digest: digest_value(definition).expect("definition digest"),
        target: target_selecting(selection),
        descriptor_digest: target_descriptor_advertising(
            selection.map_or_else(Vec::new, |value| vec![value.to_owned()]),
        )
        .digest()
        .expect("descriptor digest"),
        catalog_revision: CATALOG_REVISION.to_owned(),
    }
}

/// One submission harness: the service under test, the store it settles into,
/// the execution-port hit counter, and the exact request to submit.
pub(crate) struct Fixture {
    pub(crate) service: Arc<ManagementService>,
    pub(crate) store: Arc<dyn WorkflowStore>,
    pub(crate) submissions: Arc<AtomicUsize>,
    pub(crate) request: RunRequest,
}

pub(crate) fn fixture(
    capability: Arc<dyn CapabilityPort>,
    definition: &Value,
    request_id: &str,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_selecting(capability, definition, request_id, None)
}

/// The same harness for a request that carries a target-level inference-profile
/// selection, with a target that advertises exactly that selection — the shape
/// the served runtime publishes for its one reviewed provider-session adapter.
pub(crate) fn fixture_with_selection(
    catalogs: Vec<InferenceProfileCatalog>,
    definition: &Value,
    request_id: &str,
    selection: &str,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_selecting(
        CatalogCapabilityDouble::advertising(catalogs, vec![selection.to_owned()]),
        definition,
        request_id,
        Some(selection),
    )
}

pub(crate) fn fixture_selecting(
    capability: Arc<dyn CapabilityPort>,
    definition: &Value,
    request_id: &str,
    selection: Option<&str>,
) -> Result<Fixture, Box<dyn std::error::Error>> {
    let admission = binding_selecting(definition, request_id, selection);
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition.clone()),
        artifact_id: None,
        instance_id: admission.target.instance_id.clone(),
        profile: admission.target.execution_profile.clone(),
        admission: Some(admission),
    };
    // The service digests the request exactly as the caller supplied it, before
    // the resolved provenance replaces the target-level selection.
    let request_digest = digest_value(&serde_json::to_value(&request)?)?;
    let store: Arc<dyn WorkflowStore> = Arc::new(MemoryWorkflowStore::new());
    let (execution, submissions) = RecordingExecutionPort::new(Arc::clone(&store), request_digest);
    let service = Arc::new(
        ManagementService::new(Arc::clone(&store))
            .with_capability_port(capability)
            .with_definition_port(Arc::new(DefinitionDouble))
            .with_context_owner_port(Arc::new(FakeContextOwner))
            .with_execution_port(execution),
    );
    Ok(Fixture {
        service,
        store,
        submissions,
        request,
    })
}

/// A second submission of the same definition under a fresh request identity,
/// with a target admission bound to that identity. Reusing the first request's
/// binding would be refused for identity mismatch before any catalog check.
pub(crate) fn request_with_identity(base: &RunRequest, request_id: &str) -> RunRequest {
    let definition = base
        .definition
        .clone()
        .expect("fixture request carries a definition");
    RunRequest {
        request_id: request_id.to_owned(),
        admission: Some(binding(&definition, request_id)),
        ..base.clone()
    }
}
