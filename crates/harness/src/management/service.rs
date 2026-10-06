// SPDX-License-Identifier: MIT

use std::sync::Arc;

use serde_json::{Value, json};

use super::auth::AuthContext;
use super::authoring::{AuthoringStore, MemoryAuthoringStore};
use super::authoring_inference::{
    AuthoringInferenceJournal, AuthoringInferencePort, UnavailableAuthoringInferenceJournal,
    UnavailableAuthoringInferencePort,
};
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

#[path = "service_authoring_inference.rs"]
mod authoring_inference_ops;
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
#[path = "service_seed_v2.rs"]
mod seed_v2;
#[path = "service_seed_v2_support.rs"]
mod seed_v2_support;
#[path = "service_submission.rs"]
mod submission;
#[path = "service_support.rs"]
mod support;
#[path = "service_target_admission.rs"]
mod target_admission;
#[path = "service_unavailable.rs"]
mod unavailable;

pub use super::inference_profile_revision::{
    InferenceProfileRevisionJournal, InferenceProfileRevisionRequest,
    UnavailableInferenceProfileRevisionJournal,
};
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
            // An accepted edit must publish a new version, and a version is a
            // revision identity, so re-using one is a conflict with the
            // journal's held revisions rather than a store outage.
            "inference_profile_revision_duplicate" => ErrorClass::Conflict,
            "redaction_required" => ErrorClass::Forbidden,
            _ => ErrorClass::Store,
        };
        Self::new(class, error.code, error.message)
    }
}

#[path = "service_contracts.rs"]
mod service_contracts;
pub use execution_types::{
    CommandApplication, CommandContext, RunAdmission, RunReservation, WorkflowExecutionPort,
};
pub use service_contracts::{
    CapabilityPort, ContextInspectionPort, ContextInspectionResult, DefinitionPort, DiffResult,
    InspectionResult, ProviderSessionInspectionPort, ProviderSessionInspectionResult, ReplayResult,
    ValidationResult, WorkflowReplayPort,
};
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
    seed_derivation_keys: Option<Arc<dyn super::SeedDerivationKeyAuthority>>,
    process_lifecycle: Arc<dyn super::lifecycle::ProcessLifecyclePort>,
    lifecycle_intents: Option<Arc<std::sync::Mutex<super::lifecycle_intent::LifecycleIntentStore>>>,
    journal: Arc<dyn InferenceProfileRevisionJournal>,
    authoring_inference_provider: Arc<dyn AuthoringInferencePort>,
    authoring_inference_journal: Arc<dyn AuthoringInferenceJournal>,
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
            seed_derivation_keys: None,
            process_lifecycle: Arc::new(super::lifecycle::UnavailableProcessLifecyclePort),
            lifecycle_intents: None,
            journal: Arc::new(UnavailableInferenceProfileRevisionJournal),
            authoring_inference_provider: Arc::new(UnavailableAuthoringInferencePort),
            authoring_inference_journal: Arc::new(UnavailableAuthoringInferenceJournal),
        }
    }

    pub fn with_execution_port(mut self, port: Arc<dyn WorkflowExecutionPort>) -> Self {
        self.execution = port;
        self
    }

    /// Attaches an immutable, service-only versioned seed key authority. A
    /// missing authority leaves v2 derive-once requests typed unavailable.
    pub fn with_seed_derivation_key_authority(
        mut self,
        authority: Arc<dyn super::SeedDerivationKeyAuthority>,
    ) -> Self {
        self.seed_derivation_keys = Some(authority);
        self
    }

    pub(super) fn seed_derivation_keys(&self) -> Option<&dyn super::SeedDerivationKeyAuthority> {
        self.seed_derivation_keys.as_deref()
    }

    pub fn with_replay_port(mut self, port: Arc<dyn WorkflowReplayPort>) -> Self {
        self.replay = port;
        self
    }

    pub fn with_capability_port(mut self, port: Arc<dyn CapabilityPort>) -> Self {
        self.capabilities = port;
        self
    }

    /// Attaches the server-owned CAS revision journal that
    /// `POST /v1/inference-profiles/{profile_id}/revisions` appends to.
    ///
    /// Without it the route is composed but unavailable: an edit is refused
    /// rather than accepted into a process-local map an operator cannot see.
    pub fn with_inference_profile_revision_journal(
        mut self,
        journal: Arc<dyn InferenceProfileRevisionJournal>,
    ) -> Self {
        self.journal = journal;
        self
    }

    /// Attaches the single bounded proposal source.
    pub fn with_authoring_inference_port(mut self, port: Arc<dyn AuthoringInferencePort>) -> Self {
        self.authoring_inference_provider = port;
        self
    }

    /// Attaches the server-owned authoring-operation journal. Without it the
    /// route is composed but unavailable, so no proposal is accepted into a
    /// process-local state an operator cannot see.
    pub fn with_authoring_inference_journal(
        mut self,
        journal: Arc<dyn AuthoringInferenceJournal>,
    ) -> Self {
        self.authoring_inference_journal = journal;
        self
    }
}
