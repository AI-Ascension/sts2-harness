// SPDX-License-Identifier: MIT

use super::super::authoring::{AuthoringStore, PublishResult};
use super::super::contract_authoring::{StudioDefinitionRecord, StudioDraftRecord};
use super::*;

pub struct UnavailableAuthoringStore;

impl AuthoringStore for UnavailableAuthoringStore {
    fn list_definitions(&self) -> Result<Vec<StudioDefinitionRecord>, StoreError> {
        Err(StoreError::new(
            "authoring_store_unavailable",
            "Studio authoring persistence is not injected",
        ))
    }

    fn get_draft(&self, _draft_id: &str) -> Result<Option<StudioDraftRecord>, StoreError> {
        Err(StoreError::new(
            "authoring_store_unavailable",
            "Studio authoring persistence is not injected",
        ))
    }

    fn create_draft(
        &self,
        _draft: StudioDraftRecord,
        _mutation_id: &str,
    ) -> Result<StudioDraftRecord, StoreError> {
        Err(StoreError::new(
            "authoring_store_unavailable",
            "Studio authoring persistence is not injected",
        ))
    }

    fn save_draft(
        &self,
        _draft_id: &str,
        _expected_revision: u64,
        _expected_etag: &str,
        _mutation_id: &str,
        _document: Value,
        _layout: Value,
    ) -> Result<StudioDraftRecord, StoreError> {
        Err(StoreError::new(
            "authoring_store_unavailable",
            "Studio authoring persistence is not injected",
        ))
    }

    fn publish_draft(
        &self,
        _draft_id: &str,
        _expected_revision: u64,
        _expected_etag: &str,
        _expected_definition_digest: &str,
    ) -> Result<PublishResult, StoreError> {
        Err(StoreError::new(
            "authoring_store_unavailable",
            "Studio authoring persistence is not injected",
        ))
    }
}

pub struct UnavailableDefinitionPort;
pub struct UnavailableContextInspectionPort;
pub struct UnavailableExecutionPort;
pub struct UnavailableReplayPort;
pub struct UnavailableCapabilityPort;

impl ContextInspectionPort for UnavailableContextInspectionPort {
    fn inspect(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextInspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "context_inspection_port_unavailable",
            "context inspection is not attached to this workflow owner",
        ))
    }
}

impl DefinitionPort for UnavailableDefinitionPort {
    fn validate(
        &self,
        _definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Err(ManagementError::unavailable(
            "definition_port_unavailable",
            "workflow definition compiler/validator is not injected",
        ))
    }

    fn inspect(&self, _definition: &Value) -> Result<InspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "definition_port_unavailable",
            "workflow definition compiler/validator is not injected",
        ))
    }

    fn diff(
        &self,
        _old_definition: &Value,
        _new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        Err(ManagementError::unavailable(
            "definition_port_unavailable",
            "workflow definition compiler/validator is not injected",
        ))
    }
}

impl WorkflowExecutionPort for UnavailableExecutionPort {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        Err(ManagementError::unavailable(
            "execution_port_unavailable",
            "workflow execution authority is not injected",
        ))
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::unavailable(
            "execution_port_unavailable",
            "workflow execution authority is not injected",
        ))
    }
}

impl WorkflowReplayPort for UnavailableReplayPort {
    fn replay(
        &self,
        _request: &ReplayRequest,
        _snapshot: &RunSnapshot,
        _events: &[RunEvent],
    ) -> Result<ReplayResult, ManagementError> {
        Err(ManagementError::unavailable(
            "replay_port_unavailable",
            "offline replay reducer is not injected",
        ))
    }
}

impl CapabilityPort for UnavailableCapabilityPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Err(ManagementError::unavailable(
            "capability_port_unavailable",
            "capability authority is not injected",
        ))
    }
}
