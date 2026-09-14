// SPDX-License-Identifier: MIT

use std::sync::Arc;

use super::super::context_owner::{
    ContextBindingCatalog, ContextBindingRequest, ContextOwnerBinding, ContextOwnerPort,
};
use super::support::authorize;
use super::{AuthContext, ManagementError, ManagementService, WorkflowExecutionPort};

impl ManagementService {
    /// Sets the execution adapter and attaches the current dispatch-time context
    /// owner, so composition order cannot drop the binding.
    pub fn with_execution_port(mut self, port: Arc<dyn WorkflowExecutionPort>) -> Self {
        port.attach_context_owner(Arc::clone(&self.context_owner));
        self.execution = port;
        self
    }

    pub fn with_context_owner_port(mut self, port: Arc<dyn ContextOwnerPort>) -> Self {
        // Live execution binds each context-bound node at dispatch, so the same
        // owner must also reach the execution adapter. The default trait method
        // is a no-op for adapters that do not bind live context.
        self.execution.attach_context_owner(Arc::clone(&port));
        self.context_owner = port;
        self
    }

    pub fn context_owner_port(&self) -> &dyn ContextOwnerPort {
        self.context_owner.as_ref()
    }

    /// Returns the caller-scoped, authoritative context-binding catalog. Only
    /// bounded, redacted metadata is exposed; context bytes remain behind the
    /// owner port.
    pub fn context_owner_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<ContextBindingCatalog, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        let catalog = self.context_owner.catalog(actor)?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Establishes an owner-issued binding for one context-bound invocation.
    /// Only bounded identities are exchanged; no context bytes or effects are
    /// reachable from this path.
    pub fn bind_context(
        &self,
        actor: &AuthContext,
        request: ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        authorize(actor, "workflow:read", Some(&request.workflow_run_id))?;
        let binding = self.context_owner.bind(actor, &request)?;
        // The owner response must be the exact binding for the requested
        // invocation; a miscorrelated owner response fails closed.
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
