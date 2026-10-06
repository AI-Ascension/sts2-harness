// SPDX-License-Identifier: MIT

mod auth;
mod authoring;
mod authoring_inference;
pub use authoring_inference::{
    AuthoringInferenceBegin, AuthoringInferenceCandidate, AuthoringInferenceJournal,
    AuthoringInferencePort, AuthoringInferenceProviderRequest, MemoryAuthoringInferenceJournal,
    SqliteAuthoringInferenceJournal, UnavailableAuthoringInferenceJournal,
    UnavailableAuthoringInferencePort, authoring_inference_operation_id,
};
mod cli;
mod context_binding_history;
mod context_owner;
pub use context_binding_history::{
    RECORDED_CONTEXT_BINDING_VIEW_SCHEMA, RecordedContextBinding, RecordedContextBindingView,
};
mod contract;
mod contract_authoring;
mod contract_authoring_inference;
mod contract_seed_v2;
mod http;
mod inference_profile_binding;
pub use inference_profile_binding::{
    InferenceProfileBinding, InferenceProfileBindingSet, resolve_definition,
    resolve_definition_sites,
};
mod inference_profile_catalog;
pub use inference_profile_catalog::{
    INFERENCE_PROFILE_BINDINGS_SCHEMA_VERSION, INFERENCE_PROFILE_PROVENANCE_PREFIX,
    InferenceProfilePin, InferenceProfileRef, LiveInferenceProfileCatalogPort,
    is_provenance_reference,
};
mod inference_profile_revision;
pub use inference_profile_revision::{
    INFERENCE_PROFILE_REVISION_SCHEMA_VERSION, InferenceProfileRevisionJournal,
    InferenceProfileRevisionRequest, InferenceProfileRevisionResponse,
    MemoryInferenceProfileRevisionJournal, RevisionAppendOutcome,
    SqliteInferenceProfileRevisionJournal, derive_inference_profile_revision,
};
mod lifecycle;
mod lifecycle_intent;
mod lifecycle_readiness;
mod live_workflow;
mod provider_policy;
mod provider_session_inspection;
mod readiness_wait;
mod save_profile_setup;
mod seed_key;
mod seed_v2_crypto;
mod service;
mod store;
mod synthetic_context_owner;
pub use synthetic_context_owner::SyntheticContextOwnerPort;
mod synthetic_inference_profiles;
pub use synthetic_inference_profiles::{
    SYNTHETIC_INFERENCE_OWNER_ID, synthetic_inference_profile_catalog,
};
mod workflow_ports;
mod workflow_ports_capability;
mod workflow_ports_context;
mod workflow_ports_definition;
mod workflow_ports_execution;
mod workflow_ports_execution_inmemory;
mod workflow_ports_execution_persistent;
mod workflow_ports_replay;
mod workflow_ports_support;

pub use cli::run_cli;

pub use auth::{
    AuthContext, AuthError, Authenticator, EnvironmentAuthenticator, StaticAuthenticator,
};
pub use authoring::{AuthoringStore, MemoryAuthoringStore, PublishResult};
pub use context_owner::{
    CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA, CONTEXT_OWNER_BINDING_SCHEMA_VERSION,
    CONTEXT_OWNER_CATALOG_SCHEMA_VERSION, CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA,
    CONTEXT_OWNER_DRAFT_PATCH_SCHEMA_VERSION, CONTEXT_OWNER_DRAFT_REQUEST_SCHEMA_VERSION,
    CONTEXT_OWNER_DRAFT_SCHEMA_VERSION, CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA,
    CONTEXT_OWNER_ITEMS_SCHEMA_VERSION, CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION,
    CONTEXT_OWNER_MUTATION_RECEIPT_SCHEMA_VERSION, CONTEXT_OWNER_PREVIEW_REQUEST_SCHEMA_VERSION,
    CONTEXT_OWNER_PREVIEW_SCHEMA_VERSION, CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION,
    CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION, CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION,
    CONTEXT_OWNER_PUBLISHED_SOURCES_VIEW_SCHEMA_VERSION, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION,
    CONTEXT_OWNER_RECEIPT_V1_SCHEMA_VERSION, CONTEXT_OWNER_REVISION_SCHEMA_VERSION,
    CONTEXT_OWNER_SOURCE_STATUS_SCHEMA_VERSION, CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION,
    CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION, ContextBindingCatalog, ContextBindingContinuity,
    ContextBindingDescriptor, ContextBindingGrants, ContextBindingOperation, ContextBindingRequest,
    ContextBindingSource, ContextBindingState, ContextControlCommand, ContextControlCommandKind,
    ContextControlReceipt, ContextControlReceiptRecovery, ContextEffectiveLimits,
    ContextOwnerAssociationView, ContextOwnerBinding, ContextOwnerControlLimits,
    ContextOwnerDraftCreateRequest, ContextOwnerDraftEnvelope, ContextOwnerDraftListView,
    ContextOwnerDraftOperation, ContextOwnerDraftPatchRequest,
    ContextOwnerDraftPublicationLookupRequest, ContextOwnerDraftPublicationReceipt,
    ContextOwnerDraftPublicationRequest, ContextOwnerEffectiveLimitsView, ContextOwnerItemView,
    ContextOwnerItemsView, ContextOwnerMutationLookupRequest, ContextOwnerMutationReceipt,
    ContextOwnerMutationRequest, ContextOwnerMutationResult, ContextOwnerPort,
    ContextOwnerPreviewEnvelope, ContextOwnerPreviewRequest, ContextOwnerPublishedSourcesView,
    ContextOwnerRenderRequest, ContextOwnerRevisionEnvelope, ContextOwnerRevisionPage,
    ContextOwnerSourceStatus, ContextRenderSource, ContextRenderSourceIdentity,
    ContextSourceAdoptionRequest, ContextSourcePublication, ContextSourceUpload,
    MAX_CONTEXT_BINDINGS, MAX_CONTEXT_NODE_KINDS, MAX_CONTEXT_OPERATIONS,
    MAX_CONTEXT_OWNER_DRAFT_OPERATIONS, MAX_CONTEXT_OWNER_PAGE_SIZE, MAX_CONTEXT_SOURCES,
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
    INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION, INFERENCE_PROFILE_SCHEMA_VERSION,
    InferenceProfileBudgets, InferenceProfileCatalog, InferenceProfileContinuity,
    InferenceProfileDescriptor, InferenceProfileGrants, InferenceProfileState, InspectRequest,
    InspectResponse, MANAGEMENT_SCHEMA_VERSION, MAX_CONNECTIONS, MAX_EVENTS_PER_PAGE,
    MAX_HEADER_BYTES, MAX_IDENTIFIER_BYTES, MAX_INFERENCE_INPUT_BYTES, MAX_INFERENCE_OUTPUT_TOKENS,
    MAX_INFERENCE_PROFILES, MAX_INFERENCE_PROVIDER_CALLS, MAX_JSON_BYTES, MAX_JSON_DEPTH,
    MAX_JSON_ITEMS, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, MAX_STORE_BYTES, MAX_STRING_BYTES,
    OutputFormat, PROVIDER_SESSION_LIST_SCHEMA_VERSION,
    PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION, PROVIDER_SESSION_POLICY_VIEW_SCHEMA_VERSION,
    PendingOperation, PendingOperationState, PersistedCommand, PersistedRun, PersistedStore,
    ProviderSessionBindingSummary, ProviderSessionListResponse, ProviderSessionListValue,
    ProviderSessionOperationSummary, ProviderSessionPolicyAdoptImportedRequest,
    ProviderSessionPolicyApprovalRequest, ProviderSessionPolicyBindingMetadata,
    ProviderSessionPolicyCommandResponse, ProviderSessionPolicyHistoryMetadata,
    ProviderSessionPolicyProposalMetadata, ProviderSessionPolicyViewResponse,
    ProviderSessionPolicyViewValue, REPLAY_SCHEMA_VERSION, REQUEST_DEADLINE_MILLIS,
    RUN_SCHEMA_VERSION, RecoveryAdmission, ReplayDivergence, ReplayResponse, RunEvent, RunRequest,
    RunSnapshot, RunSubmissionResponse, RunTargetConfiguration, STATUS_SCHEMA_VERSION,
    StatusResponse, SubmissionIndex, TARGET_ADMISSION_SCHEMA_VERSION,
    TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionBinding, TargetAdmissionRequest,
    TargetAvailability, TargetCatalogResponse, TargetDescriptor, TargetPreflightResponse,
    ValidateRequest, ValidateResponse, WorkflowRunStatus, decode_strict, decode_value,
    digest_value, inference_catalog_digest, validate_digest, validate_identifier,
};
pub use contract_authoring::{
    STUDIO_SCHEMA_VERSION, StudioCreateDraftRequest, StudioDefinitionRecord,
    StudioDefinitionsResponse, StudioDraftConflict, StudioDraftRecord, StudioPublishDraftRequest,
    StudioPublishResponse, StudioSaveDraftRequest,
};
pub use contract_authoring_inference::*;
pub use contract_seed_v2::{
    SeedBindingReadbackV2, SeedBindingStateV2, SeedModeV2, SeedRequestV2,
    SeededRunSubmissionResponseV2, WORKFLOW_RUN_REQUEST_V2_SCHEMA,
    WORKFLOW_RUN_SUBMISSION_V2_SCHEMA, WORKFLOW_SEED_BINDING_V2_SCHEMA,
    WORKFLOW_SEED_REQUEST_V2_SCHEMA, WorkflowRunRequestV2,
};
pub use http::{
    ClientResponse, HttpError, HttpLimits, ManagementClient, ManagementFailurePort,
    ManagementFailureSink, ManagementServer, ServerConfig, ServerHandle,
};
pub use lifecycle::{
    LaunchProfileId, LifecycleAction, LifecycleClassification, LifecycleCommand,
    LifecycleCommandResponse, LifecycleFailure, LifecycleOperationState, LifecycleOperationView,
    LifecycleProcessIdentity, LifecycleState, LifecycleTarget, MAX_LIFECYCLE_COMMAND_BYTES,
    MAX_LIFECYCLE_COMMAND_ID_BYTES, PROCESS_LIFECYCLE_COMMAND_SCHEMA_VERSION,
    PROCESS_LIFECYCLE_CONTRACT, PROCESS_LIFECYCLE_STATUS_SCHEMA_VERSION,
    ProcessLifecycleCapability, ProcessLifecyclePort, StopMode, UnavailableProcessLifecyclePort,
    validate_lifecycle_command,
};
pub use lifecycle_intent::{
    LifecycleIntent, LifecycleIntentStore, MAX_LIFECYCLE_INTENTS, ProcessLifecycleOwner,
};
pub use lifecycle_readiness::{
    GameplayReadinessEvidence, LaunchAcknowledgement, LifecycleReadiness, ReadinessError,
    ReadinessObservation,
};
pub use live_workflow::{
    AdmittedInferenceProfileBinding, AdmittedInferenceProfileDispatch, BoundaryCaptureSink,
    EpisodeRuntimeSession, LIVE_WORKFLOW_CAPABILITY, LIVE_WORKFLOW_PROFILE,
    LiveContextObservationPort, LiveContextRenderPort, LiveProviderSessionAdmission,
    LiveProviderSessionFactory, LiveRuntimeSessionFactory, LiveTargetCatalogPort,
    LiveWorkflowExecutionPort, LiveWorkflowFactory, LiveWorkflowOptions, LiveWorkflowSession,
    LiveWorkflowSessionFactory, ProductionLiveWorkflowSessionFactory, RuntimeAuthorityBinding,
    live_run_id, live_store, live_store_with_provider_policy,
    live_store_with_provider_policy_and_command_port,
};
pub use provider_policy::{
    DurableProviderSessionPolicyCommandPort, ProviderSessionPolicyCommandPort,
    ProviderSessionPolicyOwnerCommand, ProviderSessionPolicyOwnerCommandResult,
    UnavailableProviderSessionPolicyCommandPort,
};
pub use provider_session_inspection::{
    ProviderSessionBrokerInspectionPort, ProviderSessionPolicyOwnerPort,
};
pub use readiness_wait::{
    MilestoneObservation, READINESS_CONTRACT_VERSION, ReadinessExpiry, ReadinessMilestone,
    ReadinessProgress, ReadinessTarget, ReadinessTerminal, ReadinessWait, ReadinessWaitError,
};
pub use save_profile_setup::{
    AdmittedProfileSetup, MAX_PROFILE_ID_BYTES, PROFILE_ROUTE_CONTRACT, PROFILE_ROUTE_REVISION,
    PROFILE_SETUP_SCHEMA_VERSION, ProfileBaselineFence, ProfileGrant, ProfileReadback,
    ProfileSetupError, ProfileSetupGrants, ProfileSetupOperation, ProfileSetupOperationDocument,
    ProfileSetupRequest, VerifiedProfileReadback, admit_profile_setup, is_instance_identity,
    is_profile_identity,
};
pub use seed_key::{
    FileSeedDerivationKeyAuthority, SeedDerivationKeyAuthority, SeedKeyError, SeedKeyHandle,
    SeedKeyIdentity,
};
pub use service::{
    CapabilityPort, CommandApplication, CommandContext, ContextInspectionPort,
    ContextInspectionResult, DefinitionPort, DiffResult, InspectionResult, LiveProviderPolicyPort,
    ManagementError, ManagementService, MemoryPolicyOwnerManagementPort,
    ProviderSessionInspectionPort, ProviderSessionInspectionResult, ProviderSessionPolicyBinding,
    ReplayResult, RunAdmission, RunReservation, UnavailableAuthoringStore,
    UnavailableCapabilityPort, UnavailableContextInspectionPort, UnavailableDefinitionPort,
    UnavailableExecutionPort, UnavailableLiveProviderPolicyPort,
    UnavailableMemoryPolicyOwnerManagementPort, UnavailableProviderSessionInspectionPort,
    UnavailableReplayPort, ValidationResult, WorkflowExecutionPort, WorkflowReplayPort,
};
pub use store::{
    CommandAcceptance, CommandApplication as StoreCommandApplication, FileWorkflowStore,
    MemoryWorkflowStore, SeedBindingLookup, SeedBindingRecord, SeedOperationLookup,
    SeedOperationRecord, SqliteWorkflowStore, StoreError, SubmissionLookup, WorkflowStore,
};
pub use workflow_ports::{synthetic_file_store, synthetic_sqlite_store, synthetic_store};

#[path = "serving.rs"]
mod serving;
pub use serving::{
    ServedOwnerPorts, ServedOwnerServices, serve_live, serve_live_with_lifecycle,
    serve_live_with_lifecycle_and_owner_services, serve_live_with_provider_policy,
    serve_live_with_provider_policy_and_context_owner,
    serve_live_with_provider_policy_commands_and_context_owner,
};
