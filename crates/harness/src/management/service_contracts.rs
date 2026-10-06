// SPDX-License-Identifier: MIT

use super::*;

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
    ) -> Result<super::super::contract::TargetCatalogResponse, ManagementError> {
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
    ) -> Result<Option<super::super::contract::InferenceProfileCatalog>, ManagementError> {
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
