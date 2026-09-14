// SPDX-License-Identifier: MIT

//! Immutable context binding request and owner binding records.

use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextBindingRequest {
    pub workflow_run_id: String,
    pub definition_digest: String,
    pub instance_id: String,
    pub graph_id: String,
    pub node_id: String,
    pub node_execution_id: String,
    pub node_kind: String,
    pub context_ref: String,
    pub binding_id: String,
    pub binding_version: u64,
    pub binding_digest: String,
}

impl ContextBindingRequest {
    /// Validates the immutable identity sent to the authoritative owner.
    ///
    /// The owner receives this value before it can attach any lease or
    /// context-control authority. Keeping the validation on the exported
    /// request type prevents adapters from accepting an underspecified or
    /// cross-namespace request merely because their local implementation
    /// happens to tolerate it.
    pub fn validate(&self) -> Result<(), ManagementError> {
        if self.binding_version == 0 {
            return Err(ManagementError::invalid(
                "context_binding_request_version",
                "context binding request version must be positive",
            ));
        }
        for (field, value) in [
            ("workflow_run_id", self.workflow_run_id.as_str()),
            ("context_instance_id", self.instance_id.as_str()),
            ("context_graph_id", self.graph_id.as_str()),
            ("context_node_id", self.node_id.as_str()),
            ("context_node_execution_id", self.node_execution_id.as_str()),
            ("context_node_kind", self.node_kind.as_str()),
            ("context_ref", self.context_ref.as_str()),
            ("context_binding_id", self.binding_id.as_str()),
        ] {
            validate_identifier(field, value)?;
        }
        validate_digest("definition_digest", &self.definition_digest)?;
        validate_digest("context_binding_digest", &self.binding_digest)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextOwnerBinding {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub invocation_id: String,
    pub binding_id: String,
    pub binding_version: u64,
    pub binding_digest: String,
    pub context_ref: String,
    pub instance_id: String,
    pub node_kind: String,
    pub state: ContextBindingState,
    pub workflow_run_id: String,
    pub definition_digest: String,
    pub graph_id: String,
    pub node_id: String,
    pub node_execution_id: String,
    pub boundary: ContextBoundary,
    pub lease_epoch: u64,
    pub snapshot_id: String,
    pub approved_revision_id: String,
    pub plan_epoch: u64,
    pub grants: ContextBindingGrants,
    pub continuity: ContextBindingContinuity,
}

impl ContextOwnerBinding {
    pub fn validate(&self, snapshot: Option<&RunSnapshot>) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_BINDING_SCHEMA_VERSION
            || self.binding_version == 0
            || self.lease_epoch == 0
            || self.plan_epoch == 0
        {
            return Err(ManagementError::invalid(
                "context_owner_binding_invalid",
                "context owner binding is outside its bounds",
            ));
        }
        for (field, value) in [
            ("context_owner_id", self.owner_id.as_str()),
            ("context_owner_version", self.owner_version.as_str()),
            ("context_invocation_id", self.invocation_id.as_str()),
            ("context_binding_id", self.binding_id.as_str()),
            ("context_ref", self.context_ref.as_str()),
            ("context_instance_id", self.instance_id.as_str()),
            ("context_node_kind", self.node_kind.as_str()),
            ("workflow_run_id", self.workflow_run_id.as_str()),
            ("context_graph_id", self.graph_id.as_str()),
            ("context_node_id", self.node_id.as_str()),
            ("context_node_execution_id", self.node_execution_id.as_str()),
            ("context_snapshot_id", self.snapshot_id.as_str()),
            ("context_revision_id", self.approved_revision_id.as_str()),
        ] {
            validate_identifier(field, value)?;
        }
        validate_digest("context_binding_digest", &self.binding_digest)?;
        validate_digest("definition_digest", &self.definition_digest)?;
        if self.workflow_run_id != self.boundary.run_id
            || self.workflow_run_id
                != snapshot.map_or(self.workflow_run_id.as_str(), |value| {
                    value.workflow_run_id.as_str()
                })
            || self.definition_digest
                != snapshot.map_or(self.definition_digest.as_str(), |value| {
                    value.definition_digest.as_str()
                })
        {
            return Err(ManagementError::conflict(
                "context_owner_scope_mismatch",
                "context owner binding is not attached to the admitted workflow run",
            ));
        }
        if let Some(snapshot) = snapshot
            && (self.graph_id != snapshot.cursor.graph_id
                || self.node_id != snapshot.cursor.node_id
                || self.node_execution_id != snapshot.cursor.node_execution_id)
        {
            return Err(ManagementError::conflict(
                "context_owner_cursor_mismatch",
                "context owner binding is not attached to the admitted workflow cursor",
            ));
        }
        if self.boundary.controller_epoch == 0 {
            return Err(ManagementError::invalid(
                "context_owner_controller_epoch",
                "context owner controller epoch must be positive",
            ));
        }
        validate_boundary(&self.boundary)?;
        validate_grants(&self.grants)?;
        if matches!(self.state, ContextBindingState::Available) && !self.grants.metadata_read {
            return Err(ManagementError::invalid(
                "context_owner_metadata_grant",
                "an available owner binding must expose metadata scope",
            ));
        }
        Ok(())
    }

    /// Validates that an owner response is bound to the exact request that
    /// selected it. A valid standalone binding is not sufficient: identity
    /// fields must not be silently rebound to another run, graph or node.
    pub fn validate_for_request(
        &self,
        request: &ContextBindingRequest,
    ) -> Result<(), ManagementError> {
        request.validate()?;
        self.validate(None)?;
        for (code, expected, actual) in [
            (
                "context_owner_binding_run_mismatch",
                request.workflow_run_id.as_str(),
                self.workflow_run_id.as_str(),
            ),
            (
                "context_owner_binding_definition_mismatch",
                request.definition_digest.as_str(),
                self.definition_digest.as_str(),
            ),
            (
                "context_owner_binding_graph_mismatch",
                request.graph_id.as_str(),
                self.graph_id.as_str(),
            ),
            (
                "context_owner_binding_node_mismatch",
                request.node_id.as_str(),
                self.node_id.as_str(),
            ),
            (
                "context_owner_binding_execution_mismatch",
                request.node_execution_id.as_str(),
                self.node_execution_id.as_str(),
            ),
            (
                "context_owner_binding_id_mismatch",
                request.binding_id.as_str(),
                self.binding_id.as_str(),
            ),
            (
                "context_owner_binding_digest_mismatch",
                request.binding_digest.as_str(),
                self.binding_digest.as_str(),
            ),
            (
                "context_owner_binding_ref_mismatch",
                request.context_ref.as_str(),
                self.context_ref.as_str(),
            ),
            (
                "context_owner_binding_instance_mismatch",
                request.instance_id.as_str(),
                self.instance_id.as_str(),
            ),
            (
                "context_owner_binding_kind_mismatch",
                request.node_kind.as_str(),
                self.node_kind.as_str(),
            ),
        ] {
            if expected != actual {
                return Err(ManagementError::conflict(
                    code,
                    "context owner binding does not match the requested invocation identity",
                ));
            }
        }
        if self.binding_version != request.binding_version {
            return Err(ManagementError::conflict(
                "context_owner_binding_version_mismatch",
                "context owner binding version does not match the request",
            ));
        }
        if self.boundary.run_id != request.workflow_run_id {
            return Err(ManagementError::conflict(
                "context_owner_binding_boundary_mismatch",
                "context owner binding boundary is not attached to the requested run",
            ));
        }
        if !matches!(self.state, ContextBindingState::Available) {
            return Err(ManagementError::capability(
                "context_binding_unavailable",
                "context owner returned a non-available binding",
            ));
        }
        Ok(())
    }
}

impl ContextBindingDescriptor {
    /// Validates an owner-issued binding against the immutable descriptor that
    /// admitted it. Returned authority may narrow the descriptor, but it may
    /// never add grants or continuity guarantees the catalog did not publish.
    pub fn validate_binding(&self, binding: &ContextOwnerBinding) -> Result<(), ManagementError> {
        self.validate()?;
        binding.validate(None)?;
        if !self.supports(&binding.context_ref, &binding.node_kind)
            || self.binding_id != binding.binding_id
            || self.version != binding.binding_version
            || self.digest != binding.binding_digest
        {
            return Err(ManagementError::conflict(
                "context_owner_binding_descriptor_mismatch",
                "context owner binding does not match its catalog descriptor",
            ));
        }
        if (binding.grants.metadata_read && !self.grants.metadata_read)
            || (binding.grants.content_read && !self.grants.content_read)
            || (binding.grants.edit && !self.grants.edit)
            || (binding.grants.control && !self.grants.control)
        {
            return Err(ManagementError::capability(
                "context_owner_binding_grant_escalation",
                "context owner binding grants exceed its catalog descriptor",
            ));
        }
        if (binding.continuity.survives_controller_restart
            && !self.continuity.survives_controller_restart)
            || (binding.continuity.receipt_recovery && !self.continuity.receipt_recovery)
            || (binding.continuity.provider_session_continuity
                && !self.continuity.provider_session_continuity)
        {
            return Err(ManagementError::capability(
                "context_owner_binding_continuity_escalation",
                "context owner binding continuity exceeds its catalog descriptor",
            ));
        }
        Ok(())
    }
}
