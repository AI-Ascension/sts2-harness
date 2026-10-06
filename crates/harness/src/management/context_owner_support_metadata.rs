// SPDX-License-Identifier: MIT

impl ContextEffectiveLimits {
    /// The selected limits a managed render must respect.
    ///
    /// The advertised values are validated against the harness maxima when the descriptor is
    /// validated, so this conversion cannot widen a bound; it narrows the renderer's outer
    /// harness-maxima check to what this owner/profile actually accepts.
    #[must_use]
    pub fn render_limits(&self) -> ContextRenderLimits {
        ContextRenderLimits {
            max_items: self.max_items as usize,
            max_notes: self.max_notes as usize,
            max_context_bytes: self.max_context_bytes as usize,
            max_objective_bytes: self.max_objective_bytes as usize,
            output_reserve_bytes: self.output_reserve_bytes.map(|reserve| reserve as usize),
        }
    }
}

/// Internal owner response for exact historical receipt recovery. HTTP exposes
/// only `receipt`; `binding` is retained here to validate the historical scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextControlReceiptRecovery {
    pub binding: ContextOwnerBinding,
    pub receipt: ContextControlReceipt,
}

/// HTTP-visible schema for the bounded current-association projection.
pub const CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA: &str =
    "ascension.harness.context-owner-association-view.v1";

/// Bounded, versioned projection of the authoritative context owner's current
/// binding for one workflow run.
///
/// Observation only. The projected grants, epochs and continuity flags are the
/// owner's assertions about the current binding; they confer no harness-issued
/// control authority and no current control or execution permission. The
/// originating subject is not projected and no content bytes are included.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContextOwnerAssociationView {
    pub schema_version: String,
    pub binding: ContextOwnerBinding,
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
        || limits.max_control_events > MAX_CONTROL_EVENTS
        || limits.output_reserve_bytes.is_some_and(|reserve| {
            reserve == 0
                || reserve > crate::context_memory::MAX_PREPARED_OUTPUT_RESERVE_BYTES as u64
        })
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
