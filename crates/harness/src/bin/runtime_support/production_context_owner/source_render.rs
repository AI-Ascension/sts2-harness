// SPDX-License-Identifier: MIT

use super::*;

#[path = "source_render_capture.rs"]
mod capture;

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

    fn render_source_for_decision_with_config(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &sts2_harness::management::ContextOwnerControlLimits,
        input: &sts2_harness::DecisionInput,
        context_ref: &str,
        config: &sts2_harness::ExoConfig,
    ) -> Result<ContextRenderSource, ManagementError> {
        self.capture_trusted_render_context(
            actor,
            request,
            definition_digest,
            binding,
            control_limits,
            input,
            context_ref,
            config,
        )?;
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

include!("source_render_resolution.rs");
