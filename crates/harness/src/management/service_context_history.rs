// SPDX-License-Identifier: MIT

use super::super::RecordedContextBinding;
use super::support::authorize;
use super::{
    AuthContext, ContextInspectionPort, ManagementError, ManagementService, validate_identifier,
};
use std::sync::Arc;

impl ManagementService {
    pub fn with_context_inspection_port(mut self, port: Arc<dyn ContextInspectionPort>) -> Self {
        self.context_inspection = port;
        self
    }

    /// Opts into bounded retention of owner binding metadata with command results.
    /// No content bytes are retained. Only stores supporting atomic history can
    /// enable this; existing synthetic/file/memory compositions remain unchanged.
    pub fn with_context_binding_history(mut self) -> Result<Self, ManagementError> {
        if !self.store.supports_context_binding_history() {
            return Err(ManagementError::unavailable(
                "context_history_unavailable",
                "workflow store does not support atomic context binding history",
            ));
        }
        self.context_binding_history = true;
        Ok(self)
    }

    /// Reads one historical invocation for its original subject with current
    /// workflow-read permission. Does not contact or attach an owner, refresh
    /// epochs, authorize control, or reinterpret the current cursor.
    pub fn recorded_context_binding(
        &self,
        actor: &AuthContext,
        run_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<RecordedContextBinding>, ManagementError> {
        validate_identifier("run_id", run_id)?;
        validate_identifier("node_execution_id", node_execution_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        if !self.context_binding_history {
            return Err(ManagementError::unavailable(
                "context_history_unavailable",
                "context binding history is not enabled",
            ));
        }
        let record = self
            .store
            .recorded_context_binding(run_id, node_execution_id)?;
        if let Some(record) = &record
            && record.subject != actor.subject
        {
            return Err(ManagementError::forbidden(
                "context_history_subject",
                "context binding history belongs to another subject",
            ));
        }
        Ok(record)
    }
}
