// SPDX-License-Identifier: MIT

use super::*;

impl LiveContextRenderPort for Owner {
    fn render_source_for_decision(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &sts2_harness::DecisionInput,
        context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError> {
        self.resolve_render_source(
            actor,
            request,
            definition_digest,
            binding,
            control_limits,
            input,
            context_ref,
        )
    }

    fn assert_render_source_current(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &sts2_harness::DecisionInput,
        context_ref: &str,
        expected: &ContextRenderSourceIdentity,
    ) -> Result<(), ManagementError> {
        let current = self.resolve_render_source(
            actor,
            request,
            definition_digest,
            binding,
            control_limits,
            input,
            context_ref,
        )?;
        if &current.identity != expected || current.now >= current.valid_until {
            return Err(ManagementError::conflict(
                "context_render_source_stale",
                "active source, binding, runtime observation, or selected limits changed",
            ));
        }
        Ok(())
    }

    fn render_required(&self) -> bool {
        self.configuration.render_required
    }
}

impl Owner {
    #[allow(clippy::too_many_arguments)]
    fn resolve_render_source(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        runtime_binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &sts2_harness::DecisionInput,
        context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError> {
        if !self.configuration.render_required
            || !actor.can("workflow:control")
            || !actor.can_run(&runtime_binding.run_id)
        {
            return Err(ManagementError::forbidden(
                "context_render_forbidden",
                "actor cannot use managed context for this workflow run",
            ));
        }
        let expected_run = run_id(request, definition_digest)?;
        if runtime_binding.run_id != expected_run
            || runtime_binding.instance_id != request.instance_id
            || runtime_binding.lease_id.is_empty()
            || runtime_binding.lease_epoch == 0
        {
            return Err(ManagementError::conflict(
                "context_render_runtime_scope",
                "render runtime authority does not match the admitted workflow run",
            ));
        }
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| {
                ManagementError::conflict(
                    "context_render_catalog_stale",
                    "managed rendering requires the current legal-action catalog",
                )
            })?;
        let observed_digest = sts2_harness::sha256_hex(
            serde_json::to_vec(input.observation.fair_play().as_value()).map_err(|error| {
                ManagementError::invalid("context_observation_encode", error.to_string())
            })?,
        );
        let catalog_digest = legal_catalog_digest(&input.legal_actions)?;
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&expected_run).ok_or_else(|| {
            ManagementError::unavailable(
                "context_render_source_unavailable",
                "current owner observation is unavailable",
            )
        })?;
        self.validate_control_limits(actor, control_limits)?;
        if entry.actor != actor.subject
            || entry.definition_digest != definition_digest
            || entry.runtime_instance_id != runtime_binding.instance_id
            || entry.runtime_lease_id != runtime_binding.lease_id
            || entry.runtime_lease_epoch != runtime_binding.lease_epoch
            || entry.admitted_control_limits != *control_limits
            || entry.catalog_generation != Some(input.observation.generation())
            || entry.authority.state().boundary.state_id != input.observation.state_id()
            || entry.authority.state().boundary.generation != input.observation.generation()
            || entry.authority.state().boundary.observation_sha256 != observed_digest
            || entry.authority.state().boundary.catalog_sha256 != catalog_digest
        {
            return Err(ManagementError::conflict(
                "context_render_boundary_stale",
                "decision input is not the current owner observation and legal-action catalog",
            ));
        }
        let binding_request = entry.binding_request.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "context_render_binding_unavailable",
                "current context-bound invocation has not been bound",
            )
        })?;
        if binding_request.context_ref != context_ref
            || binding_request.workflow_run_id != expected_run
            || binding_request.definition_digest != definition_digest
            || binding_request.instance_id != runtime_binding.instance_id
            || binding_request.node_kind != "decide"
        {
            return Err(ManagementError::conflict(
                "context_render_binding_mismatch",
                "current owner binding does not admit this decision context",
            ));
        }
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
        self.validate_control_limits_in_catalog(&catalog, control_limits)?;
        let binding = self.binding_for_request(binding_request, entry, &catalog)?;
        let view = ContextOwnerEffectiveLimitsView::compose(&catalog, &binding)?;
        if !binding.grants.content_read {
            return Err(ManagementError::capability(
                "context_render_content_unavailable",
                "current owner binding does not grant managed content access",
            ));
        }
        let state = entry.authority.state();
        let (active, source) = entry
            .store
            .active_context_source(&state.active_revision_id)
            .map_err(|error| {
                ManagementError::unavailable("context_render_source_store", error.to_string())
            })?
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "context_render_source_unavailable",
                    "no explicitly adopted context source is active for this run",
                )
            })?;
        let advertised = self.advertised_source(&active.source_id)?;
        if active.version != advertised.version
            || active.digest != advertised.digest
            || active.active_revision_id != state.active_revision_id
            || !binding_request.context_ref.eq(context_ref)
        {
            return Err(ManagementError::conflict(
                "context_render_source_stale",
                "active source is not the exact owner-advertised source for this revision",
            ));
        }
        let now = unix_time()?;
        let valid_until = source_valid_until(&source.document);
        if now >= valid_until {
            return Err(ManagementError::conflict(
                "context_render_source_expired",
                "active context source contains expired selected content",
            ));
        }
        let source_identity = ContextRenderSourceIdentity {
            owner_id: binding.owner_id.clone(),
            owner_version: binding.owner_version.clone(),
            catalog_digest: catalog.catalog_digest,
            binding_id: binding.binding_id.clone(),
            binding_version: binding.binding_version,
            binding_digest: binding.binding_digest.clone(),
            invocation_id: binding.invocation_id.clone(),
            instance_id: binding.instance_id.clone(),
            lease_id: runtime_binding.lease_id.clone(),
            lease_epoch: runtime_binding.lease_epoch,
            active_revision_id: state.active_revision_id.clone(),
            source_id: active.source_id.clone(),
            source_version: active.version,
            source_digest: active.digest.clone(),
            boundary: state.boundary.clone(),
        };
        Ok(ContextRenderSource {
            source_id: active.source_id,
            source_version: active.version,
            source_digest: active.digest,
            active_revision_id: active.active_revision_id,
            boundary: state.boundary.clone(),
            limits: view.render_limits(),
            document: source.document,
            now,
            valid_until,
            identity: source_identity,
        })
    }
}
