// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde_json::{Value, json};

use super::auth::AuthContext;
use super::authoring::{AuthoringStore, MemoryAuthoringStore};
use super::contract::{
    AuthoritySummary, CONTEXT_ASSOCIATION_SCHEMA_VERSION, CapabilityResponse, CleanupState,
    CommandRequest, CommandResponse, ContextAssociation, ContextAssociationContext,
    ContextAvailability, ContextCaptureEvidence, ContextInspectionCapabilities,
    ContextWorkflowIdentity, ContractError, Diagnostic, DiffRequest, DiffResponse, ErrorBody,
    ErrorClass, ErrorResponse, EventPage, ExportRequest, ExportResponse, HealthResponse,
    InspectRequest, InspectResponse, MANAGEMENT_SCHEMA_VERSION, OutputFormat,
    REPLAY_SCHEMA_VERSION, RUN_SCHEMA_VERSION, RecoveryAdmission, ReplayDivergence, ReplayRequest,
    ReplayResponse, RunEvent, RunRequest, RunSnapshot, RunSubmissionResponse,
    STATUS_SCHEMA_VERSION, StatusResponse, ValidateRequest, ValidateResponse, WorkflowRunStatus,
    digest_value, schema_is, validate_digest, validate_identifier,
};
use super::store::{
    CommandAcceptance, CommandApplication as StoredCommandApplication, FileWorkflowStore,
    MemoryWorkflowStore, StoreError, SubmissionLookup, WorkflowStore,
};

#[path = "service_authoring.rs"]
mod authoring_ops;
#[path = "service_ops.rs"]
mod ops;
#[path = "service_read.rs"]
mod read;
#[path = "service_support.rs"]
mod support;
#[path = "service_unavailable.rs"]
mod unavailable;

pub use unavailable::{
    UnavailableAuthoringStore, UnavailableCapabilityPort, UnavailableContextInspectionPort,
    UnavailableDefinitionPort, UnavailableExecutionPort, UnavailableReplayPort,
};

/// A stable management error. The HTTP and CLI adapters map `class` to their
/// respective status/exit-code contracts without exposing port implementation
/// details or private payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagementError {
    pub class: ErrorClass,
    pub code: String,
    pub message: String,
}

impl ManagementError {
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::InvalidInput, code, message)
    }

    pub fn capability(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Capability, code, message)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Conflict, code, message)
    }

    pub fn forbidden(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Forbidden, code, message)
    }

    pub fn unresolved(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Unresolved, code, message)
    }

    pub fn unavailable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Unavailable, code, message)
    }

    pub fn budget(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Budget, code, message)
    }

    pub fn store(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Store, code, message)
    }

    pub fn replay(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Replay, code, message)
    }

    pub fn authentication(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Authentication, code, message)
    }

    pub fn error_response(&self) -> ErrorResponse {
        ErrorResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            error: ErrorBody {
                class: self.class.clone(),
                code: self.code.clone(),
                message: self.message.clone(),
            },
        }
    }
}

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

pub trait WorkflowExecutionPort: Send + Sync {
    fn submit(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError>;

    fn apply_command(&self, context: CommandContext)
    -> Result<CommandApplication, ManagementError>;
}

#[derive(Clone, Debug)]
pub struct RunAdmission {
    pub snapshot: RunSnapshot,
    pub initial_events: Vec<RunEvent>,
}

#[derive(Clone, Debug)]
pub struct CommandContext {
    pub request: CommandRequest,
    pub snapshot: RunSnapshot,
    pub actor: AuthContext,
}

#[derive(Clone, Debug)]
pub struct CommandApplication {
    pub snapshot: RunSnapshot,
    pub outcome: super::contract::CommandOutcome,
    pub reason_code: String,
}

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

pub struct ManagementService {
    store: Arc<dyn WorkflowStore>,
    authoring: Arc<dyn AuthoringStore>,
    definitions: Arc<dyn DefinitionPort>,
    execution: Arc<dyn WorkflowExecutionPort>,
    replay: Arc<dyn WorkflowReplayPort>,
    capabilities: Arc<dyn CapabilityPort>,
    context_inspection: Arc<dyn ContextInspectionPort>,
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
        }
    }

    pub fn in_memory() -> Self {
        Self::new(Arc::new(MemoryWorkflowStore::new()))
            .with_authoring_store(Arc::new(MemoryAuthoringStore::new()))
    }

    pub fn file_store(store: FileWorkflowStore) -> Self {
        Self::new(Arc::new(store))
    }

    pub fn with_definition_port(mut self, port: Arc<dyn DefinitionPort>) -> Self {
        self.definitions = port;
        self
    }

    pub fn with_authoring_store(mut self, store: Arc<dyn AuthoringStore>) -> Self {
        self.authoring = store;
        self
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

    pub fn with_context_inspection_port(mut self, port: Arc<dyn ContextInspectionPort>) -> Self {
        self.context_inspection = port;
        self
    }
}
