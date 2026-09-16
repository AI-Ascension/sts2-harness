// SPDX-License-Identifier: MIT

mod auth;
mod authoring;
mod cli;
mod context_binding_history;
mod context_owner;
pub use context_binding_history::{
    RECORDED_CONTEXT_BINDING_VIEW_SCHEMA, RecordedContextBinding, RecordedContextBindingView,
};
mod contract;
mod contract_authoring;
mod http;
mod live_workflow;
mod provider_session_inspection;
mod service;
mod store;
mod synthetic_context_owner;
pub use synthetic_context_owner::SyntheticContextOwnerPort;
mod workflow_ports;

pub use cli::run_cli;

pub use auth::{
    AuthContext, AuthError, Authenticator, EnvironmentAuthenticator, StaticAuthenticator,
};
pub use authoring::{AuthoringStore, MemoryAuthoringStore, PublishResult};
pub use context_owner::{
    CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA, CONTEXT_OWNER_BINDING_SCHEMA_VERSION,
    CONTEXT_OWNER_CATALOG_SCHEMA_VERSION, CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA,
    CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, CONTEXT_OWNER_RECEIPT_V1_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingOperation, ContextBindingRequest, ContextBindingSource,
    ContextBindingState, ContextControlCommand, ContextControlCommandKind, ContextControlReceipt,
    ContextEffectiveLimits, ContextOwnerAssociationView, ContextOwnerBinding,
    ContextOwnerEffectiveLimitsView, ContextOwnerPort, MAX_CONTEXT_BINDINGS,
    MAX_CONTEXT_NODE_KINDS, MAX_CONTEXT_OPERATIONS, MAX_CONTEXT_SOURCES,
    UnavailableContextOwnerPort, compose_context_owner_binding,
};
pub use contract::ReplayRequest as ManagementReplayRequest;
pub use contract::{
    AuthoritySummary, Budget, CAPABILITIES_SCHEMA_VERSION, CONTEXT_ASSOCIATION_SCHEMA_VERSION,
    CapabilityResponse, CleanupState, CommandKind, CommandOutcome, CommandParameters,
    CommandRequest, CommandResponse, ContextAssociation, ContextAssociationContext,
    ContextAvailability, ContextCaptureEvidence, ContextCaptureMode, ContextCaptureState,
    ContextInspectionCapabilities, ContextWorkflowIdentity, ContractError, Cursor, Diagnostic,
    DiagnosticSeverity, DiffRequest, DiffResponse, EVENT_SCHEMA_VERSION, EXPORT_SCHEMA_VERSION,
    ErrorBody, ErrorClass, ErrorResponse, EventClassification, EventGap, EventPage, EventPayload,
    EventType, ExecutionMode, ExportRequest, ExportResponse, GameOutcome, HealthResponse,
    InspectRequest, InspectResponse, MANAGEMENT_SCHEMA_VERSION, MAX_CONNECTIONS,
    MAX_EVENTS_PER_PAGE, MAX_HEADER_BYTES, MAX_IDENTIFIER_BYTES, MAX_JSON_BYTES, MAX_JSON_DEPTH,
    MAX_JSON_ITEMS, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, MAX_STORE_BYTES, MAX_STRING_BYTES,
    OutputFormat, PROVIDER_SESSION_LIST_SCHEMA_VERSION, PendingOperation, PendingOperationState,
    PersistedCommand, PersistedRun, PersistedStore, ProviderSessionBindingSummary,
    ProviderSessionListResponse, ProviderSessionListValue, ProviderSessionOperationSummary,
    REPLAY_SCHEMA_VERSION, REQUEST_DEADLINE_MILLIS, RUN_SCHEMA_VERSION, RecoveryAdmission,
    ReplayDivergence, ReplayResponse, RunEvent, RunRequest, RunSnapshot, RunSubmissionResponse,
    RunTargetConfiguration, STATUS_SCHEMA_VERSION, StatusResponse, SubmissionIndex,
    TARGET_ADMISSION_SCHEMA_VERSION, TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionBinding,
    TargetAdmissionRequest, TargetAvailability, TargetCatalogResponse, TargetDescriptor,
    TargetPreflightResponse, ValidateRequest, ValidateResponse, WorkflowRunStatus, decode_strict,
    decode_value, digest_value, validate_digest, validate_identifier,
};
pub use contract_authoring::{
    STUDIO_SCHEMA_VERSION, StudioCreateDraftRequest, StudioDefinitionRecord,
    StudioDefinitionsResponse, StudioDraftConflict, StudioDraftRecord, StudioPublishDraftRequest,
    StudioPublishResponse, StudioSaveDraftRequest,
};
pub use http::{
    ClientResponse, HttpError, HttpLimits, ManagementClient, ManagementServer, ServerConfig,
    ServerHandle,
};
pub use live_workflow::{
    EpisodeRuntimeSession, LIVE_WORKFLOW_CAPABILITY, LIVE_WORKFLOW_PROFILE,
    LiveWorkflowExecutionPort, LiveWorkflowFactory, LiveWorkflowOptions, LiveWorkflowSession,
    LiveWorkflowSessionFactory, live_store,
};
pub use provider_session_inspection::ProviderSessionBrokerInspectionPort;
pub use service::{
    CapabilityPort, CommandApplication, CommandContext, ContextInspectionPort,
    ContextInspectionResult, DefinitionPort, DiffResult, InspectionResult, ManagementError,
    ManagementService, ProviderSessionInspectionPort, ProviderSessionInspectionResult,
    ReplayResult, RunAdmission, RunReservation, UnavailableAuthoringStore,
    UnavailableCapabilityPort, UnavailableContextInspectionPort, UnavailableDefinitionPort,
    UnavailableExecutionPort, UnavailableProviderSessionInspectionPort, UnavailableReplayPort,
    ValidationResult, WorkflowExecutionPort, WorkflowReplayPort,
};
pub use store::{
    CommandAcceptance, FileWorkflowStore, MemoryWorkflowStore, SqliteWorkflowStore, StoreError,
    WorkflowStore,
};
pub use workflow_ports::{synthetic_file_store, synthetic_sqlite_store, synthetic_store};
