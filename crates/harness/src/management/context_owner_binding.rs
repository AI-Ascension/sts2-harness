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
}
