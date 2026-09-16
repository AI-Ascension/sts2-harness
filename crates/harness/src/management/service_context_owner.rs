// SPDX-License-Identifier: MIT

use std::sync::Arc;

use super::super::context_owner::{
    CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA, ContextControlCommand, ContextControlReceipt,
    ContextOwnerAssociationView, ContextOwnerEffectiveLimitsView,
};
use super::super::context_owner::{
    ContextBindingCatalog, ContextBindingRequest, ContextOwnerBinding, ContextOwnerPort,
};
use super::support::authorize;
use super::{AuthContext, ManagementError, ManagementService, RunSnapshot, validate_identifier};
use crate::context_control::{
    ContextBoundary, ContextDraft, ContextItem, ContextRenderError, ControlAuthority,
    ManagedRenderInput, PreparedContext,
};
use crate::exo::ExoConfig;
use std::collections::BTreeMap;

impl ManagementService {
    pub fn with_context_owner_port(mut self, port: Arc<dyn ContextOwnerPort>) -> Self {
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

    /// Recovers the receipt the authoritative owner already issued for `command`,
    /// for a caller whose reply was lost or ambiguous.
    ///
    /// This path never re-issues, re-applies or infers an effect. It requires
    /// current scoped `workflow:read` for the run, the owner's current
    /// association for that run, and an owner that advertises
    /// `receipt_recovery` in its binding continuity. A recovered receipt must
    /// satisfy exact owner/invocation/binding/command identity before it is
    /// returned, so a receipt for one command cannot be replayed as another.
    pub fn recover_context_control_receipt(
        &self,
        actor: &AuthContext,
        run_id: &str,
        command: &ContextControlCommand,
    ) -> Result<ContextControlReceipt, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let binding = self.current_context_binding(actor, &snapshot)?;
        if !binding.continuity.receipt_recovery {
            return Err(ManagementError::unavailable(
                "context_control_receipt_recovery_unsupported",
                "the authoritative context owner does not advertise control receipt recovery",
            ));
        }
        let receipt = self
            .context_owner
            .control_receipt(actor, &binding, command)?
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_control_receipt_not_recorded",
                    "the context owner has no recorded receipt for the supplied command",
                )
            })?;
        receipt.validate_for(&binding, command)?;
        Ok(receipt)
    }

    /// Resolves the authoritative owner's current association for one run and
    /// fails closed if the owner answers for a different run.
    fn current_context_binding(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let binding = self.context_owner.association(actor, snapshot)?;
        binding.validate(None)?;
        if binding.workflow_run_id != snapshot.workflow_run_id {
            return Err(ManagementError::conflict(
                "context_binding_mismatch",
                "context owner returned a binding for a different workflow run",
            ));
        }
        Ok(binding)
    }

    /// The authoritative context owner's **current** association for one run, as
    /// a versioned read-only projection. This is the current binding, not the
    /// historical invocation history, and it confers no harness-issued control
    /// authority: the projected grants and epochs are owner assertions.
    ///
    /// An unattached or unavailable owner stays explicitly unavailable rather
    /// than being reported as "no association".
    pub fn current_context_owner_association(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextOwnerAssociationView, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let binding = self.current_context_binding(actor, &snapshot)?;
        Ok(ContextOwnerAssociationView {
            schema_version: CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA.to_owned(),
            binding,
        })
    }

    /// The effective context limits the authoritative owner currently admits
    /// for one run, as a versioned read-only projection.
    ///
    /// This composes the owner's **current** binding with the catalog descriptor
    /// that admits it, so the reported values are the ones published for this
    /// binding and adapter/model revision rather than the portable schema maxima
    /// or the harness maxima. The values are owner assertions and confer no
    /// execution authority; an unattached owner stays explicitly unavailable.
    pub fn current_context_effective_limits(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextOwnerEffectiveLimitsView, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        self.composed_context_limits(actor, run_id)
    }

    /// Prepares the managed render for a run's current binding under the limits
    /// that binding advertised.
    ///
    /// This is the production render point for a composed, authenticated owner:
    /// the run's current binding and its admitting descriptor are composed
    /// fail-closed exactly as the read-only limits projection composes them, and
    /// the resulting **selected** limits are enforced by the renderer before any
    /// inference or retention. A draft the harness maxima could prepare but this
    /// owner does not accept is refused with
    /// `ManagementError` code `context_render_limit_exceeded` naming the limit;
    /// an unattached owner or an unusable binding stays explicitly unavailable
    /// rather than falling back to the harness maxima.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_context_render(
        &self,
        actor: &AuthContext,
        run_id: &str,
        boundary: &ContextBoundary,
        request: ManagedRenderInput,
        draft: &ContextDraft,
        registry: &BTreeMap<String, ContextItem>,
        config: &ExoConfig,
        now: u64,
    ) -> Result<PreparedContext, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:control", Some(run_id))?;
        let (view, binding) = self.current_context_composition(actor, run_id)?;
        if boundary != &binding.boundary {
            return Err(ManagementError::conflict(
                "context_render_boundary_mismatch",
                "render boundary is not the current authoritative owner boundary",
            ));
        }
        view.prepare_managed_render(boundary, request, draft, registry, config, now)
            .map_err(context_render_error)
    }

    /// Applies a run's selected control-transition bound to a harness control
    /// authority.
    ///
    /// The control-transition path records pause, resume, commit and boundary
    /// transitions through this authority. The owner's advertised
    /// `max_control_events` narrows how many transitions may be retained, so a
    /// binding that accepts fewer transitions than the harness maximum refuses
    /// with `context_control_events_exhausted` instead of saturating silently.
    /// Composition failures (unattached owner, unusable binding, tampered
    /// descriptor) are refused before any bound is applied.
    pub fn bind_context_control_authority(
        &self,
        actor: &AuthContext,
        run_id: &str,
        authority: ControlAuthority,
    ) -> Result<ControlAuthority, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:control", Some(run_id))?;
        let (view, binding) = self.current_context_composition(actor, run_id)?;
        if authority.state().boundary != binding.boundary {
            return Err(ManagementError::conflict(
                "context_control_boundary_mismatch",
                "control authority boundary is not the current authoritative owner boundary",
            ));
        }
        view.bind_control_authority(authority)
    }

    /// Composes the run's current binding with the catalog that admits it.
    ///
    /// Shared by the read-only effective-limits projection and the two
    /// enforcement entry points above, so what a caller can observe and what a
    /// caller must respect come from one fail-closed composition.
    fn composed_context_limits(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextOwnerEffectiveLimitsView, ManagementError> {
        self.current_context_composition(actor, run_id)
            .map(|(view, _)| view)
    }

    /// Resolves both halves of the current owner composition. Callers which
    /// execute a render or control transition must retain the binding so they
    /// can prove their supplied boundary is the same boundary the owner
    /// currently attached to this run.
    fn current_context_composition(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<(ContextOwnerEffectiveLimitsView, ContextOwnerBinding), ManagementError> {
        if !self.context_owner.is_available() {
            return Err(ManagementError::unavailable(
                "context_owner_unavailable",
                "selected context limits require an attached authoritative context owner",
            ));
        }
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let binding = self.current_context_binding(actor, &snapshot)?;
        let catalog = self.context_owner.catalog(actor)?;
        catalog.validate()?;
        let view = ContextOwnerEffectiveLimitsView::compose(&catalog, &binding)?;
        Ok((view, binding))
    }
}

/// Classifies a render refusal for the management surface.
///
/// A selected-limit refusal keeps its own code so a caller can tell "the owner
/// selected less than this" apart from "the harness bound was exceeded".
fn context_render_error(error: ContextRenderError) -> ManagementError {
    match error {
        ContextRenderError::ExceedsSelectedLimit(limit) => ManagementError::capability(
            "context_render_limit_exceeded",
            format!("prepared context exceeds the selected owner limit: {limit}"),
        ),
        other => ManagementError::invalid("context_render_invalid", other.to_string()),
    }
}
