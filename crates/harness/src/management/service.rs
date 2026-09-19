// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde_json::{Value, json};

use super::auth::AuthContext;
use super::authoring::{AuthoringStore, MemoryAuthoringStore};
use super::context_owner::ContextOwnerPort;
use super::contract::{
    AuthoritySummary, CONTEXT_ASSOCIATION_SCHEMA_VERSION, CapabilityResponse, CleanupState,
    CommandRequest, CommandResponse, ContextAssociation, ContextAssociationContext,
    ContextAvailability, ContextCaptureEvidence, ContextInspectionCapabilities,
    ContextWorkflowIdentity, ContractError, Diagnostic, DiffRequest, DiffResponse, ErrorBody,
    ErrorClass, ErrorResponse, EventPage, ExportRequest, ExportResponse, HealthResponse,
    InspectRequest, InspectResponse, MANAGEMENT_SCHEMA_VERSION, OutputFormat,
    PROVIDER_SESSION_LIST_SCHEMA_VERSION, PROVIDER_SESSION_POLICY_VIEW_SCHEMA_VERSION,
    ProviderSessionBindingSummary, ProviderSessionListResponse, ProviderSessionListValue,
    ProviderSessionOperationSummary, ProviderSessionPolicyCommandResponse,
    ProviderSessionPolicyViewResponse, REPLAY_SCHEMA_VERSION, RUN_SCHEMA_VERSION,
    RecoveryAdmission, ReplayDivergence, ReplayRequest, ReplayResponse, RunEvent, RunRequest,
    RunSnapshot, RunSubmissionResponse, RunTargetConfiguration, STATUS_SCHEMA_VERSION,
    StatusResponse, TARGET_ADMISSION_SCHEMA_VERSION, TargetAdmissionBinding,
    TargetAdmissionRequest, TargetAvailability, TargetCatalogResponse, TargetDescriptor,
    TargetPreflightResponse, ValidateRequest, ValidateResponse, WorkflowRunStatus, digest_value,
    schema_is, validate_digest, validate_identifier,
};
use super::store::{
    CommandAcceptance, CommandApplication as StoredCommandApplication, FileWorkflowStore,
    MemoryWorkflowStore, StoreError, SubmissionLookup, WorkflowStore,
};

#[path = "service_authoring.rs"]
mod authoring_ops;
#[path = "service_constructors.rs"]
mod constructors;
#[path = "service_context_history.rs"]
mod context_history;
#[path = "service_context_owner.rs"]
mod context_owner_port;
#[path = "service_effective_limits.rs"]
mod effective_limits_ops;
#[path = "service_execution.rs"]
mod execution_types;
#[path = "service_inference_profile.rs"]
mod inference_profile_ops;
#[path = "service_ops_lifecycle.rs"]
mod lifecycle_ops;
#[path = "service_live_provider_policy.rs"]
mod live_provider_policy;
#[path = "service_memory_policy_owner.rs"]
mod memory_policy_owner_ops;
#[path = "service_ops.rs"]
mod ops;
#[path = "service_process_lifecycle.rs"]
mod process_lifecycle_ops;
#[path = "service_provider_policy.rs"]
mod provider_policy_ops;
#[path = "service_provider_session.rs"]
mod provider_session_support;
#[path = "service_read.rs"]
mod read;
#[path = "service_submission.rs"]
mod submission;
#[path = "service_support.rs"]
mod support;
#[path = "service_target_admission.rs"]
mod target_admission;
#[path = "service_unavailable.rs"]
mod unavailable;

pub use live_provider_policy::{
    LiveProviderPolicyPort, ProviderSessionPolicyBinding, UnavailableLiveProviderPolicyPort,
};
pub use memory_policy_owner_ops::*;
pub use provider_session_support::UnavailableProviderSessionInspectionPort;
pub use unavailable::{
    UnavailableAuthoringStore, UnavailableCapabilityPort, UnavailableContextInspectionPort,
    UnavailableDefinitionPort, UnavailableExecutionPort, UnavailableReplayPort,
};

/// Stable management errors map to status or exit code and omit private payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagementError {
    pub class: ErrorClass,
    pub code: String,
    pub message: String,
}

#[path = "service_error.rs"]
mod service_error;

impl std::fmt::Display for ManagementError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ManagementError {}

impl From<ContractError> for ManagementError {
    fn from(error: ContractError) -> Self {
        Self::invalid(error.code, error.message)
    }
}

impl From<StoreError> for ManagementError {
    fn from(error: StoreError) -> Self {
        let class = match error.code.as_str() {
            "run_not_found" | "command_not_found" => ErrorClass::InvalidInput,
            "stale_revision" | "duplicate_run" | "command_conflict" => ErrorClass::Conflict,
            "redaction_required" => ErrorClass::Forbidden,
            _ => ErrorClass::Store,
        };
        Self::new(class, error.code, error.message)
    }
}

pub trait DefinitionPort: Send + Sync {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError>;

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError>;

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationResult {
    pub definition_digest: String,
    pub compiler: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectionResult {
    pub definition_digest: String,
    pub workflow_id: Option<String>,
    pub workflow_version: Option<String>,
    pub required_capabilities: Vec<String>,
    pub graph_count: u64,
    pub node_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffResult {
    pub old_definition_digest: String,
    pub new_definition_digest: String,
    pub semantic_change: bool,
    pub changed_paths: Vec<String>,
}

pub use execution_types::{
    CommandApplication, CommandContext, RunAdmission, RunReservation, WorkflowExecutionPort,
};

pub trait WorkflowReplayPort: Send + Sync {
    fn replay(
        &self,
        request: &ReplayRequest,
        snapshot: &RunSnapshot,
        events: &[RunEvent],
    ) -> Result<ReplayResult, ManagementError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayResult {
    pub matched: bool,
    pub compared_events: u64,
    pub first_divergence: Option<ReplayDivergence>,
}

pub trait CapabilityPort: Send + Sync {
    fn capabilities(&self) -> Result<Value, ManagementError>;

    /// Returns the caller-scoped run-target catalog. This is separate from
    /// the capability value because instance identity and availability are
    /// authority-owned metadata, not workflow JSON.
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<super::contract::TargetCatalogResponse, ManagementError> {
        Err(ManagementError::unavailable(
            "target_catalog_unavailable",
            "target discovery is not attached to this workflow owner",
        ))
    }

    /// Returns the caller-scoped inference-profile catalog, or `None` when the
    /// owner serves none. An absent catalog keeps the capability-prefix
    /// admission of decision references; a served catalog is authoritative and
    /// every decision/planner reference must resolve in it before inference.
    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<Option<super::contract::InferenceProfileCatalog>, ManagementError> {
        Ok(None)
    }
}

/// The harness-owned integration boundary for context evidence. Implementations
/// receive only the authoritative workflow snapshot and return redacted,
/// scope-bound metadata. They cannot execute a provider or mutate a run.
pub trait ContextInspectionPort: Send + Sync {
    fn inspect(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ContextInspectionResult, ManagementError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextInspectionResult {
    pub context: ContextAssociationContext,
    pub capture: ContextCaptureEvidence,
    pub capabilities: ContextInspectionCapabilities,
}

/// The harness-owned boundary for provider-session metadata. Implementations
/// receive an already-authorized workflow snapshot and may return only the
/// bounded summaries below. They cannot accept native IDs, issue provider
/// commands, or make a browser-controlled cross-namespace lookup.
pub trait ProviderSessionInspectionPort: Send + Sync {
    fn list(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ProviderSessionInspectionResult, ManagementError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSessionInspectionResult {
    pub workflow_run_id: String,
    pub bindings: Vec<ProviderSessionBindingSummary>,
    pub operations: Vec<ProviderSessionOperationSummary>,
    pub next_cursor: Option<String>,
}

pub struct ManagementService {
    store: Arc<dyn WorkflowStore>,
    authoring: Arc<dyn AuthoringStore>,
    definitions: Arc<dyn DefinitionPort>,
    execution: Arc<dyn WorkflowExecutionPort>,
    replay: Arc<dyn WorkflowReplayPort>,
    capabilities: Arc<dyn CapabilityPort>,
    context_inspection: Arc<dyn ContextInspectionPort>,
    context_owner: Arc<dyn ContextOwnerPort>,
    context_binding_history: bool,
    provider_session_inspection: Arc<dyn ProviderSessionInspectionPort>,
    provider_session_policy: Arc<dyn super::provider_policy::ProviderSessionPolicyCommandPort>,
    live_provider_policy: Arc<dyn LiveProviderPolicyPort>,
    memory_policy_owner: Arc<dyn MemoryPolicyOwnerManagementPort>,
    provider_session_capabilities: Option<crate::provider_session::NativeCapabilities>,
    process_lifecycle: Arc<dyn super::lifecycle::ProcessLifecyclePort>,
    lifecycle_intents: Option<Arc<std::sync::Mutex<super::lifecycle_intent::LifecycleIntentStore>>>,
}

impl ManagementService {
    pub fn new(store: Arc<dyn WorkflowStore>) -> Self {
        Self {
            store,
            authoring: Arc::new(UnavailableAuthoringStore),
            definitions: Arc::new(UnavailableDefinitionPort),
            execution: Arc::new(UnavailableExecutionPort),
            replay: Arc::new(UnavailableReplayPort),
            capabilities: Arc::new(UnavailableCapabilityPort),
            context_inspection: Arc::new(UnavailableContextInspectionPort),
            context_owner: Arc::new(super::context_owner::UnavailableContextOwnerPort),
            context_binding_history: false,
            provider_session_inspection: Arc::new(
                provider_session_support::UnavailableProviderSessionInspectionPort,
            ),
            provider_session_policy: Arc::new(
                super::provider_policy::UnavailableProviderSessionPolicyCommandPort,
            ),
            live_provider_policy: Arc::new(live_provider_policy::UnavailableLiveProviderPolicyPort),
            memory_policy_owner: Arc::new(
                memory_policy_owner_ops::UnavailableMemoryPolicyOwnerManagementPort,
            ),
            provider_session_capabilities: None,
            process_lifecycle: Arc::new(super::lifecycle::UnavailableProcessLifecyclePort),
            lifecycle_intents: None,
        }
    }

    pub fn with_execution_port(mut self, port: Arc<dyn WorkflowExecutionPort>) -> Self {
        self.execution = port;
        self
    }

    pub fn with_replay_port(mut self, port: Arc<dyn WorkflowReplayPort>) -> Self {
        self.replay = port;
        self
    }

    pub fn with_capability_port(mut self, port: Arc<dyn CapabilityPort>) -> Self {
        self.capabilities = port;
        self
    }
}
