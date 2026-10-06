// SPDX-License-Identifier: MIT

use super::*;

impl Owner {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture_trusted_render_context(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        runtime_binding: &RuntimeAuthorityBinding,
        control_limits: &sts2_harness::management::ContextOwnerControlLimits,
        input: &sts2_harness::DecisionInput,
        context_ref: &str,
        config: &sts2_harness::ExoConfig,
    ) -> Result<(), ManagementError> {
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
            || context_ref != self.configuration.context_ref
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
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
        self.validate_control_limits_in_catalog(&catalog, control_limits)?;
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&expected_run).ok_or_else(|| {
            ManagementError::unavailable(
                "context_render_source_unavailable",
                "current owner observation is unavailable",
            )
        })?;
        if entry.actor != actor.subject
            || entry.definition_digest != definition_digest
            || entry.runtime_instance_id != runtime_binding.instance_id
            || entry.runtime_lease_id != runtime_binding.lease_id
            || entry.runtime_lease_epoch != runtime_binding.lease_epoch
            || entry.admitted_control_limits != *control_limits
            || entry.catalog_generation != Some(input.observation.generation())
            || runtime_binding.configuration_digest
                != entry.authority.state().boundary.configuration_sha256
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
        let current_binding = self.binding_for_request(binding_request, entry, &catalog)?;
        if config.revision != sts2_harness::EXO_SOURCE_REVISION
            || runtime_binding.configuration_digest != current_binding.boundary.configuration_sha256
            || runtime_binding.adapter_revision != current_binding.boundary.adapter_revision
        {
            return Err(ManagementError::conflict(
                "context_render_config_stale",
                "admitted Exo source pin or runtime configuration binding is not current",
            ));
        }
        let provider_config_digest = provider_config_digest(config)?;
        entry.trusted_render = Some(TrustedRenderContext {
            request: input.managed_render_input(),
            config: config.clone(),
            provider_config_digest,
            binding: current_binding,
            actor_subject: actor.subject.clone(),
            runtime_lease_id: runtime_binding.lease_id.clone(),
            captured_at: unix_time()?,
        });
        Ok(())
    }
}
