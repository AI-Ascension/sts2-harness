// SPDX-License-Identifier: MIT

//! Test doubles for the multi-target admission matrix (#99).
//!
//! Everything here is synthetic and labelled as such: the catalog is an
//! in-memory, per-actor `CapabilityPort` the test mutates between discovery,
//! preflight and submission; the execution port only records submissions; the
//! drifting factory replays a scripted catalog sequence into the live adapter.
//! None of it is evidence of native game, host or provider behaviour.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, Budget, CapabilityPort, CleanupState, CommandApplication, CommandContext, Cursor,
    DefinitionPort, Diagnostic, DiffResult, EVENT_SCHEMA_VERSION, EventClassification,
    EventPayload, EventType, ExecutionMode, GameOutcome, InspectionResult, LiveWorkflowSession,
    LiveWorkflowSessionFactory, ManagementError, RUN_SCHEMA_VERSION, RunAdmission, RunEvent,
    RunRequest, RunSnapshot, RunTargetConfiguration, TARGET_CATALOG_SCHEMA_VERSION,
    TargetAvailability, TargetCatalogResponse, TargetDescriptor, ValidationResult,
    WorkflowExecutionPort, WorkflowRunStatus, WorkflowStore, digest_value,
};

pub(crate) const CATALOG_REVISION: &str = "matrix.catalog.v1";
pub(crate) const ALPHA: &str = "instance-alpha";
pub(crate) const BETA: &str = "instance-beta";

/// First authorized fake target: one live profile, no optional namespaces.
pub(crate) fn alpha_descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: ALPHA.to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.alpha.v1".to_owned(),
        capability_revision: "live.capabilities.alpha.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: vec!["workflow.live".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: Vec::new(),
    }
}

/// Second authorized fake target: distinct revisions and every optional
/// namespace populated, so its exact admission cannot collide with alpha's.
pub(crate) fn beta_descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: BETA.to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned(), "live.workflow.v2".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.beta.v2".to_owned(),
        capability_revision: "live.capabilities.beta.v2".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: vec![
            "workflow.live".to_owned(),
            "workflow.context.context.live.v1".to_owned(),
        ],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: vec!["save-beta-v1".to_owned()],
        inference_profiles: vec!["inference-beta-v1".to_owned()],
    }
}

pub(crate) fn catalog(targets: Vec<TargetDescriptor>) -> TargetCatalogResponse {
    TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: CATALOG_REVISION.to_owned(),
        targets,
    }
}

/// The exact selection a consumer would derive from a served descriptor.
pub(crate) fn selection(descriptor: &TargetDescriptor, profile: &str) -> RunTargetConfiguration {
    RunTargetConfiguration {
        instance_id: descriptor.instance_id.clone(),
        execution_profile: profile.to_owned(),
        execution_mode: descriptor.execution_mode.clone(),
        workflow_revision: "0.1.0".to_owned(),
        compatibility_revision: descriptor.compatibility_revision.clone(),
        capability_revision: descriptor.capability_revision.clone(),
        game_profile: "sts2-live-v1".to_owned(),
        save_profile: descriptor.save_profiles.first().cloned(),
        inference_profile: descriptor.inference_profiles.first().cloned(),
        context_capability: descriptor
            .capabilities
            .iter()
            .find(|value| value.starts_with("workflow.context."))
            .cloned(),
        provider_capability: None,
    }
}

/// Per-actor, mutable catalog double. A subject without an entry is denied
/// discovery exactly like the production catalog denies a missing scope.
pub(crate) struct MatrixCatalog {
    catalogs: Mutex<BTreeMap<String, TargetCatalogResponse>>,
    calls: AtomicUsize,
}

impl MatrixCatalog {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            catalogs: Mutex::new(BTreeMap::new()),
            calls: AtomicUsize::new(0),
        })
    }

    pub(crate) fn serve(&self, subject: &str, catalog: TargetCatalogResponse) {
        self.catalogs
            .lock()
            .expect("catalog lock")
            .insert(subject.to_owned(), catalog);
    }

    pub(crate) fn mutate(&self, subject: &str, change: impl FnOnce(&mut TargetCatalogResponse)) {
        let mut catalogs = self.catalogs.lock().expect("catalog lock");
        let catalog = catalogs.get_mut(subject).expect("served subject");
        change(catalog);
    }

    pub(crate) fn mutate_target(
        &self,
        subject: &str,
        instance_id: &str,
        change: impl FnOnce(&mut TargetDescriptor),
    ) {
        self.mutate(subject, |catalog| {
            let target = catalog
                .targets
                .iter_mut()
                .find(|target| target.instance_id == instance_id)
                .expect("served target");
            change(target);
        });
    }

    pub(crate) fn remove_target(&self, subject: &str, instance_id: &str) {
        self.mutate(subject, |catalog| {
            catalog
                .targets
                .retain(|target| target.instance_id != instance_id);
        });
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl CapabilityPort for MatrixCatalog {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({
            "schema_version": "ascension.capabilities/v1",
            "capabilities": ["workflow.live"]
        }))
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.catalogs
            .lock()
            .expect("catalog lock")
            .get(&actor.subject)
            .cloned()
            .ok_or_else(|| {
                ManagementError::forbidden(
                    "target_scope_denied",
                    "target discovery is not available to this actor",
                )
            })
    }
}

/// Definition port that admits any JSON definition with its raw digest.
pub(crate) struct MatrixDefinitionPort;

impl DefinitionPort for MatrixDefinitionPort {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Ok(ValidationResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            compiler: "matrix-compiler.v1".to_owned(),
            diagnostics: Vec::<Diagnostic>::new(),
        })
    }

    fn inspect(&self, _definition: &Value) -> Result<InspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "inspection is not used by this test",
        ))
    }

    fn diff(&self, _old: &Value, _new: &Value) -> Result<DiffResult, ManagementError> {
        Err(ManagementError::unavailable(
            "unused",
            "diff is not used by this test",
        ))
    }
}

/// Execution port that records every submission it receives. It performs no
/// live, provider or game effect and does not declare an execution mode.
pub(crate) struct RecordingExecutionPort {
    submissions: AtomicUsize,
}

impl RecordingExecutionPort {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            submissions: AtomicUsize::new(0),
        })
    }

    pub(crate) fn submissions(&self) -> usize {
        self.submissions.load(Ordering::SeqCst)
    }
}

impl WorkflowExecutionPort for RecordingExecutionPort {
    fn submit(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        let run_id = format!("run-{}", request.request_id);
        let snapshot = RunSnapshot {
            schema_version: RUN_SCHEMA_VERSION.to_owned(),
            workflow_run_id: run_id.clone(),
            definition_digest: definition_digest.to_owned(),
            run_revision: 1,
            status: WorkflowRunStatus::Running,
            game_outcome: GameOutcome::NotTerminal,
            cursor: Cursor {
                graph_id: "graph".to_owned(),
                node_id: "node".to_owned(),
                node_execution_id: "node-exec".to_owned(),
            },
            pending_operation: None,
            budget: Budget::default(),
            cleanup: CleanupState::NotStarted,
            admission: None,
            execution_mode: None,
        };
        Ok(RunAdmission {
            initial_events: vec![RunEvent {
                schema_version: EVENT_SCHEMA_VERSION.to_owned(),
                workflow_run_id: run_id,
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
        })
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

/// Live session factory whose catalog answer is scripted per call: the n-th
/// `target_catalog` call returns `catalogs[min(n, len - 1)]`. This reaches the
/// live adapter's own re-check immediately before the session is opened.
pub(crate) struct DriftingFactory {
    pub(crate) inner: super::support::FakeFactory,
    catalogs: Vec<TargetCatalogResponse>,
    calls: AtomicUsize,
}

impl DriftingFactory {
    pub(crate) fn new(catalogs: Vec<TargetCatalogResponse>) -> Arc<Self> {
        Arc::new(Self {
            inner: super::support::FakeFactory::new(false),
            catalogs,
            calls: AtomicUsize::new(0),
        })
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LiveWorkflowSessionFactory for DriftingFactory {
    fn capabilities(&self) -> Value {
        self.inner.capabilities()
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let index = call.min(self.catalogs.len().saturating_sub(1));
        self.catalogs.get(index).cloned().ok_or_else(|| {
            ManagementError::unavailable("fake_catalog_missing", "no scripted catalog")
        })
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.inner
            .open(request, actor, definition, definition_digest)
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        definition_digest: &str,
        control_limits: Option<&sts2_harness::management::ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.inner.open_admitted(
            request,
            actor,
            definition,
            definition_digest,
            control_limits,
        )
    }
}

/// Throwaway SQLite location under the cargo target tree, removed on drop.
pub(crate) struct Database {
    directory: PathBuf,
    pub(crate) path: PathBuf,
}

impl Database {
    pub(crate) fn new() -> Self {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/management-target-admission-matrix")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).expect("test database directory");
        let path = directory.join("workflow.sqlite3");
        Self { directory, path }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub(crate) fn persisted_admission(
    store: &dyn WorkflowStore,
    run_id: &str,
) -> Option<sts2_harness::management::TargetAdmissionBinding> {
    store
        .get_run(run_id)
        .expect("store lookup")
        .and_then(|snapshot| snapshot.admission)
}
