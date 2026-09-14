// SPDX-License-Identifier: MIT

//! Control, receipt and port surface for the context owner contract.

use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextControlCommand {
    Pause {
        idempotency_key: String,
        expected_control_version: u64,
    },
    Commit {
        idempotency_key: String,
        expected_control_version: u64,
        expected_revision_id: String,
        expected_boundary: ContextBoundary,
        preview_manifest_digest: String,
        approved_manifest_digest: String,
    },
    Resume {
        idempotency_key: String,
        expected_control_version: u64,
        expected_boundary: ContextBoundary,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextControlReceipt {
    pub schema_version: String,
    pub owner_id: String,
    pub invocation_id: String,
    pub binding_id: String,
    pub binding_digest: String,
    pub command_id: String,
    pub idempotency_key: String,
    pub effect: String,
    pub control_version: u64,
    pub plan_epoch: u64,
    pub controller_epoch: u64,
    pub gate_epoch: u64,
}

impl ContextControlReceipt {
    pub fn validate_for(
        &self,
        binding: &ContextOwnerBinding,
        command: &ContextControlCommand,
    ) -> Result<(), ManagementError> {
        if self.schema_version != CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION
            || self.owner_id != binding.owner_id
            || self.invocation_id != binding.invocation_id
            || self.binding_id != binding.binding_id
            || self.binding_digest != binding.binding_digest
            || self.control_version == 0
            || self.plan_epoch == 0
            || self.controller_epoch == 0
            || self.gate_epoch == 0
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_mismatch",
                "context control receipt is not bound to the requested owner invocation",
            ));
        }
        validate_identifier("context_control_command_id", &self.command_id)?;
        validate_identifier("context_control_idempotency_key", &self.idempotency_key)?;
        validate_identifier("context_control_effect", &self.effect)?;
        validate_digest("context_binding_digest", &self.binding_digest)?;
        let (idempotency_key, expected_control_version, boundary) = match command {
            ContextControlCommand::Pause {
                idempotency_key,
                expected_control_version,
            } => (idempotency_key, *expected_control_version, None),
            ContextControlCommand::Commit {
                idempotency_key,
                expected_control_version,
                expected_boundary,
                ..
            } => (
                idempotency_key,
                *expected_control_version,
                Some(expected_boundary),
            ),
            ContextControlCommand::Resume {
                idempotency_key,
                expected_control_version,
                expected_boundary,
            } => (
                idempotency_key,
                *expected_control_version,
                Some(expected_boundary),
            ),
        };
        validate_identifier("context_control_idempotency_key", idempotency_key)?;
        if expected_control_version == 0
            || self.idempotency_key != *idempotency_key
            || self.control_version < expected_control_version
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_fence",
                "context control receipt does not satisfy the requested version fence",
            ));
        }
        if let Some(expected_boundary) = boundary {
            validate_boundary(expected_boundary)?;
            if !self.boundary_matches(expected_boundary) {
                return Err(ManagementError::conflict(
                    "context_control_receipt_boundary",
                    "context control receipt boundary does not match the request",
                ));
            }
        }
        Ok(())
    }

    fn boundary_matches(&self, boundary: &ContextBoundary) -> bool {
        self.controller_epoch == boundary.controller_epoch && self.gate_epoch == boundary.gate_epoch
    }
}

pub trait ContextOwnerPort: Send + Sync {
    fn catalog(&self, actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError>;

    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError>;

    fn association(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_association_unavailable",
            "context owner association is not attached to this workflow owner",
        ))
    }

    fn control(
        &self,
        _actor: &AuthContext,
        _binding: &ContextOwnerBinding,
        _command: &ContextControlCommand,
    ) -> Result<ContextControlReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_control_unavailable",
            "context control receipt delegation is not attached to this owner",
        ))
    }

    fn is_available(&self) -> bool {
        true
    }
}

pub struct UnavailableContextOwnerPort;

impl ContextOwnerPort for UnavailableContextOwnerPort {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_catalog_unavailable",
            "authoritative context owner catalog is not attached",
        ))
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        _request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_binding_unavailable",
            "authoritative context owner is not attached",
        ))
    }

    fn is_available(&self) -> bool {
        false
    }
}

pub(crate) fn catalog_digest(
    owner_id: &str,
    owner_version: &str,
    descriptors: &[ContextBindingDescriptor],
) -> Result<String, ManagementError> {
    let bytes = serde_json::to_vec(&(owner_id, owner_version, descriptors))
        .map_err(|error| ManagementError::invalid("context_catalog_encode", error.to_string()))?;
    Ok(sha256_hex(bytes))
}

pub(crate) fn validate_limits(limits: &ContextEffectiveLimits) -> Result<(), ManagementError> {
    if limits.max_items == 0
        || limits.max_items > MAX_CONTEXT_ITEMS as u64
        || limits.max_notes > MAX_CONTEXT_NOTES as u64
        || limits.max_context_bytes == 0
        || limits.max_context_bytes > MAX_CONTEXT_BYTES as u64
        || limits.max_objective_bytes == 0
        || limits.max_objective_bytes > MAX_OBJECTIVE_BYTES as u64
        || limits.max_control_events == 0
        || limits.max_control_events > 4096
    {
        return Err(ManagementError::invalid(
            "context_effective_limits_invalid",
            "context owner limits exceed the harness safety ceilings",
        ));
    }
    Ok(())
}

pub(crate) fn validate_grants(grants: &ContextBindingGrants) -> Result<(), ManagementError> {
    if grants.content_read && !grants.metadata_read {
        return Err(ManagementError::invalid(
            "context_grant_scope",
            "content access cannot be advertised without metadata access",
        ));
    }
    if grants.edit && !grants.content_read {
        return Err(ManagementError::invalid(
            "context_grant_scope",
            "context edit cannot be advertised without content-read scope",
        ));
    }
    if grants.control && !grants.metadata_read {
        return Err(ManagementError::invalid(
            "context_grant_scope",
            "context control cannot be advertised without metadata scope",
        ));
    }
    Ok(())
}

pub(crate) fn validate_boundary(boundary: &ContextBoundary) -> Result<(), ManagementError> {
    for (field, value) in [
        ("context_boundary_run_id", boundary.run_id.as_str()),
        ("context_boundary_episode_id", boundary.episode_id.as_str()),
        ("context_boundary_agent_id", boundary.agent_id.as_str()),
        ("context_boundary_state_id", boundary.state_id.as_str()),
        (
            "context_boundary_adapter_revision",
            boundary.adapter_revision.as_str(),
        ),
        (
            "context_boundary_model_revision",
            boundary.model_revision.as_str(),
        ),
    ] {
        validate_identifier(field, value)?;
    }
    for (field, value) in [
        (
            "context_boundary_observation_digest",
            boundary.observation_sha256.as_str(),
        ),
        (
            "context_boundary_catalog_digest",
            boundary.catalog_sha256.as_str(),
        ),
        (
            "context_boundary_configuration_digest",
            boundary.configuration_sha256.as_str(),
        ),
        (
            "context_boundary_output_schema_digest",
            boundary.output_schema_sha256.as_str(),
        ),
    ] {
        validate_digest(field, value)?;
    }
    if boundary.generation == 0
        || boundary.controller_epoch == 0
        || boundary.gate_epoch == 0
        || boundary.control_version == 0
    {
        return Err(ManagementError::invalid(
            "context_boundary_epoch",
            "context boundary generation and epochs must be positive",
        ));
    }
    Ok(())
}
