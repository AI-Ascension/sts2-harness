// SPDX-License-Identifier: MIT

//! Authenticated composition of the authoritative owner's current binding with
//! the catalog descriptor that admits it.
//!
//! A catalog publishes what an owner *advertises*; a binding is what the owner
//! *granted* for one invocation. Neither half is sufficient on its own, so both
//! are composed and cross-checked before any advertised value is observable or
//! executable. The composition is the single seam used by live admission and by
//! the read-only owner surface, so the limits a caller can observe are the
//! limits the same run was admitted under.

use super::*;
use crate::context_control::{
    ContextBoundary, ContextDraft, ContextItem, ContextRenderError, ContextRenderLimits,
    ContextRenderer, ControlAuthority, ManagedRenderInput, PreparedContext,
};
use crate::exo::ExoConfig;
use std::collections::BTreeMap;

/// HTTP-visible schema for the composed effective-limits projection.
pub const CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA: &str =
    "ascension.harness.context-owner-effective-limits-view.v1";
/// Schema for the bounded control limit preflight used before a workflow has a
/// runtime-allocated invocation identity.
pub const CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA: &str =
    "ascension.harness.context-owner-control-limits.v1";

/// Owner-published control bound admitted for a workflow definition before it
/// has a current invocation binding.
///
/// This deliberately is not a [`ContextOwnerBinding`]. A binding must name the
/// runtime-allocated run, graph, node and node-execution identities, none of
/// which exists during submission. Callers must re-resolve and compose the
/// current binding before an effect or restart recovery; this value only
/// constrains creation of the initial control authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextOwnerControlLimits {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub catalog_digest: String,
    pub max_control_events: u64,
}

impl ContextOwnerControlLimits {
    /// Selects the narrowest control-event bound among every context-bound
    /// node in a definition. A workflow can transition to any admitted node,
    /// so using the minimum prevents a later descriptor from silently
    /// widening a journal that was started before its invocation existed.
    pub fn from_descriptors(
        catalog: &ContextBindingCatalog,
        descriptors: &[&ContextBindingDescriptor],
    ) -> Result<Self, ManagementError> {
        catalog.validate()?;
        let max_control_events = descriptors
            .iter()
            .map(|descriptor| descriptor.effective_limits.max_control_events)
            .min()
            .ok_or_else(|| {
                ManagementError::capability(
                    "context_binding_unsupported",
                    "workflow has no context-bound descriptor for control preflight",
                )
            })?;
        Ok(Self {
            schema_version: CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA.to_owned(),
            owner_id: catalog.owner_id.clone(),
            owner_version: catalog.owner_version.clone(),
            catalog_digest: catalog.catalog_digest.clone(),
            max_control_events,
        })
    }
}

/// Bounded, versioned projection of the effective limits the authoritative
/// owner currently admits for one workflow run.
///
/// Observation only. The values are the owner's assertion for this binding and
/// revision; they confer no control, edit, capture or execution authority, and
/// they cannot widen the harness maxima. The originating subject is not
/// projected and no content bytes are included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContextOwnerEffectiveLimitsView {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub catalog_digest: String,
    pub binding_id: String,
    pub binding_version: u64,
    pub binding_digest: String,
    pub context_ref: String,
    pub node_kind: String,
    pub adapter_revision: String,
    pub model_revision: String,
    pub effective_limits: ContextEffectiveLimits,
}

impl ContextOwnerEffectiveLimitsView {
    /// Composes the projection from a validated catalog and the owner's current
    /// binding for one run. Every refusal below happens before a single
    /// advertised value is used:
    ///
    /// - a binding issued by another catalog owner is `context_owner_binding_foreign`;
    /// - a catalog that advertises no usable descriptor for the binding's
    ///   context/node kind fails closed rather than reporting absent limits;
    /// - a binding that is not the exact published descriptor identity
    ///   (id, version, digest) is `context_owner_binding_descriptor_mismatch`;
    /// - a binding that is not `available`, or that escalates grants or
    ///   continuity beyond that descriptor, is refused by `validate_binding`.
    ///
    /// `catalog.validate()` must be run first: it bounds every value to the
    /// harness maxima and re-derives the descriptor and catalog digests, so a
    /// tampered or oversized descriptor cannot reach this function.
    pub fn compose(
        catalog: &ContextBindingCatalog,
        binding: &ContextOwnerBinding,
    ) -> Result<Self, ManagementError> {
        let descriptor = compose_context_owner_binding(catalog, binding)?;
        Ok(Self {
            schema_version: CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA.to_owned(),
            owner_id: catalog.owner_id.clone(),
            owner_version: catalog.owner_version.clone(),
            catalog_digest: catalog.catalog_digest.clone(),
            binding_id: binding.binding_id.clone(),
            binding_version: binding.binding_version,
            binding_digest: binding.binding_digest.clone(),
            context_ref: binding.context_ref.clone(),
            node_kind: binding.node_kind.clone(),
            adapter_revision: binding.boundary.adapter_revision.clone(),
            model_revision: binding.boundary.model_revision.clone(),
            effective_limits: descriptor.effective_limits.clone(),
        })
    }

    /// The selected render limits this binding advertised.
    ///
    /// The advertised values were bounded by the harness maxima when the catalog was validated, so
    /// this conversion can only narrow the renderer's outer bound.
    #[must_use]
    pub fn render_limits(&self) -> ContextRenderLimits {
        self.effective_limits.render_limits()
    }

    /// Prepares a managed render under the limits this binding advertised.
    ///
    /// This is the production caller of `ContextRenderer::enabled_at_with_limits`: the composed,
    /// authenticated owner limits travel into the renderer, so a draft the harness could prepare
    /// but this owner does not accept is refused with `ContextRenderError::ExceedsSelectedLimit`
    /// naming the limit, before any inference or retention. The harness-maxima checks still run
    /// first and are unchanged.
    pub fn prepare_managed_render(
        &self,
        boundary: &ContextBoundary,
        request: ManagedRenderInput,
        draft: &ContextDraft,
        registry: &BTreeMap<String, ContextItem>,
        config: &ExoConfig,
        now: u64,
    ) -> Result<PreparedContext, ContextRenderError> {
        ContextRenderer::enabled_at_with_limits(
            boundary,
            request,
            draft,
            registry,
            config,
            now,
            &self.render_limits(),
        )
    }

    /// Applies the selected control-transition bound to a harness control authority.
    ///
    /// The advertised `max_control_events` narrows the authority's recorded-transition bound, so
    /// the control-transition path refuses past what this owner accepted instead of saturating
    /// silently at the harness maximum. Only a validated descriptor can reach this method; a
    /// refusal here is explicit rather than a silent clamp.
    pub fn bind_control_authority(
        &self,
        authority: ControlAuthority,
    ) -> Result<ControlAuthority, ManagementError> {
        authority
            .with_max_control_events(self.effective_limits.max_control_events)
            .map_err(|code| {
                let reason = if code == "context_control_events_exhausted" {
                    "context_control_events_exhausted"
                } else {
                    "context_control_event_limit_invalid"
                };
                ManagementError::invalid(reason, code)
            })
    }
}

/// Resolves the catalog descriptor that admits `binding`, or fails closed.
///
/// The returned descriptor is the only authority for the binding's advertised
/// limits, grants and continuity. A descriptor the owner no longer publishes,
/// a foreign owner, and any binding that is not the exact descriptor identity
/// are all refused before the descriptor's values are used.
pub fn compose_context_owner_binding<'catalog>(
    catalog: &'catalog ContextBindingCatalog,
    binding: &ContextOwnerBinding,
) -> Result<&'catalog ContextBindingDescriptor, ManagementError> {
    if binding.owner_id != catalog.owner_id || binding.owner_version != catalog.owner_version {
        return Err(ManagementError::conflict(
            "context_owner_binding_foreign",
            "context owner binding was issued by a different catalog owner",
        ));
    }
    if !matches!(binding.state, ContextBindingState::Available) {
        return Err(ManagementError::capability(
            "context_binding_unavailable",
            "context owner binding is not currently available",
        ));
    }
    let descriptor = catalog.descriptor_for(&binding.context_ref, &binding.node_kind)?;
    if descriptor.binding_id != binding.binding_id
        || descriptor.version != binding.binding_version
        || descriptor.digest != binding.binding_digest
    {
        return Err(ManagementError::conflict(
            "context_owner_binding_descriptor_mismatch",
            "context owner binding does not match the descriptor that admits it",
        ));
    }
    descriptor.validate_binding(binding)?;
    Ok(descriptor)
}
