// SPDX-License-Identifier: MIT

mod auth;
mod cli;
mod contract;
mod http;
mod service;
mod store;
mod workflow_ports;

pub use cli::run_cli;

pub use auth::{
    AuthContext, AuthError, Authenticator, EnvironmentAuthenticator, StaticAuthenticator,
};
pub use contract::ReplayRequest as ManagementReplayRequest;
pub use contract::{
    AuthoritySummary, Budget, CAPABILITIES_SCHEMA_VERSION, CapabilityResponse, CleanupState,
    CommandKind, CommandOutcome, CommandParameters, CommandRequest, CommandResponse, ContractError,
    Cursor, Diagnostic, DiagnosticSeverity, DiffRequest, DiffResponse, EVENT_SCHEMA_VERSION,
    EXPORT_SCHEMA_VERSION, ErrorBody, ErrorClass, ErrorResponse, EventClassification, EventGap,
    EventPage, EventPayload, EventType, ExportRequest, ExportResponse, GameOutcome, HealthResponse,
    InspectRequest, InspectResponse, MANAGEMENT_SCHEMA_VERSION, MAX_CONNECTIONS,
    MAX_EVENTS_PER_PAGE, MAX_HEADER_BYTES, MAX_IDENTIFIER_BYTES, MAX_JSON_BYTES, MAX_JSON_DEPTH,
    MAX_JSON_ITEMS, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, MAX_STORE_BYTES, MAX_STRING_BYTES,
    OutputFormat, PendingOperation, PendingOperationState, PersistedCommand, PersistedRun,
    PersistedStore, REPLAY_SCHEMA_VERSION, REQUEST_DEADLINE_MILLIS, RUN_SCHEMA_VERSION,
    RecoveryAdmission, ReplayDivergence, ReplayResponse, RunEvent, RunRequest, RunSnapshot,
    RunSubmissionResponse, STATUS_SCHEMA_VERSION, StatusResponse, SubmissionIndex, ValidateRequest,
    ValidateResponse, WorkflowRunStatus, decode_strict, decode_value, digest_value,
    validate_digest, validate_identifier,
};
pub use http::{
    ClientResponse, HttpError, HttpLimits, ManagementClient, ManagementServer, ServerConfig,
    ServerHandle,
};
pub use service::{
    CapabilityPort, CommandApplication, CommandContext, DefinitionPort, DiffResult,
    InspectionResult, ManagementError, ManagementService, ReplayResult, RunAdmission,
    UnavailableCapabilityPort, UnavailableDefinitionPort, UnavailableExecutionPort,
    UnavailableReplayPort, ValidationResult, WorkflowExecutionPort, WorkflowReplayPort,
};
pub use store::{
    CommandAcceptance, FileWorkflowStore, MemoryWorkflowStore, SqliteWorkflowStore, StoreError,
    WorkflowStore,
};
pub use workflow_ports::{synthetic_file_store, synthetic_sqlite_store, synthetic_store};
