// SPDX-License-Identifier: MIT

use super::super::super::context_owner::{ContextBindingRequest, ContextOwnerBinding};
use super::super::support::authorize;
use super::{ManagementError, ManagementService};

impl ManagementService {
    /// Establishes an owner-issued binding for one context-bound invocation.
    /// Only bounded identities are exchanged; no context bytes or effects are
    /// reachable from this path.
    pub fn bind_context(
        &self,
        actor: &super::AuthContext,
        request: ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        authorize(actor, "workflow:read", Some(&request.workflow_run_id))?;
        if let Some(snapshot) = self.store.get_run(&request.workflow_run_id)? {
            validate_admitted_request(&snapshot, &request)?;
        }
        let binding = self.context_owner.bind(actor, &request)?;
        if binding.workflow_run_id != request.workflow_run_id
            || binding.definition_digest != request.definition_digest
            || binding.graph_id != request.graph_id
            || binding.node_id != request.node_id
            || binding.node_execution_id != request.node_execution_id
            || binding.context_ref != request.context_ref
            || binding.binding_id != request.binding_id
            || binding.binding_version != request.binding_version
            || binding.binding_digest != request.binding_digest
        {
            return Err(ManagementError::conflict(
                "context_binding_mismatch",
                "context owner returned a binding for a different invocation",
            ));
        }
        binding.validate(None)?;
        Ok(binding)
    }
}

fn validate_admitted_request(
    snapshot: &super::super::super::contract::RunSnapshot,
    request: &ContextBindingRequest,
) -> Result<(), ManagementError> {
    if snapshot.definition_digest != request.definition_digest
        || snapshot.cursor.graph_id != request.graph_id
        || snapshot.cursor.node_id != request.node_id
        || snapshot.cursor.node_execution_id != request.node_execution_id
    {
        return Err(ManagementError::conflict(
            "context_binding_cursor_mismatch",
            "context binding request does not match the admitted workflow cursor",
        ));
    }
    if let Some(admission) = &snapshot.admission {
        admission.validate().map_err(ManagementError::from)?;
        if admission.workflow_definition_digest != snapshot.definition_digest
            || admission.target.instance_id != request.instance_id
        {
            return Err(ManagementError::conflict(
                "context_binding_instance_mismatch",
                "context binding instance does not match the admitted workflow target",
            ));
        }
    }
    Ok(())
}
