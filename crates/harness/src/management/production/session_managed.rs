// SPDX-License-Identifier: MIT

use super::*;
use crate::management::LiveContextRenderPort;
use std::sync::Arc;

impl ProductionLiveWorkflowSession {
    pub(super) fn decide_with_managed_context(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        render: Arc<dyn LiveContextRenderPort>,
    ) -> Result<crate::Decision, ManagementError> {
        let control_limits = self.context_control_limits.clone().ok_or_else(|| {
            ManagementError::capability(
                "selected_context_control_limits_required",
                "managed rendering has no admitted context control limits",
            )
        })?;
        let actor = self.actor.clone();
        let request = self.request.clone();
        let definition_digest = self.definition_digest.clone();
        let authority_binding = self.authority_binding.clone();
        let source = render.render_source_for_decision(
            &actor,
            &request,
            &definition_digest,
            &authority_binding,
            &control_limits,
            input,
            context_ref,
        )?;
        let prepared = self
            .provider_mut()?
            .prepare_managed_context(input, &source)
            .map_err(provider_error)?;
        admit_assembled_managed_input(&source.limits, prepared.provider_bytes().len())?;
        // The approval is bound and fenced before the write, so a refused or unrecordable boundary
        // stops the decision before anything reaches the provider.
        render.assert_render_source_current(
            &actor,
            &request,
            &definition_digest,
            &authority_binding,
            &control_limits,
            input,
            context_ref,
            &source.identity,
        )?;
        // The recorded release performs the exchange inside the recording write port, so the
        // approved material and the bytes the boundary wrote are one value. Everything from here on
        // may already have reached the provider, so the re-assertions below report an unresolved
        // outcome rather than a clean refusal.
        let capture = self.boundary_capture.clone();
        let decision = self.dispatch_managed_boundary(
            input,
            decision_profile_ref,
            context_ref,
            &prepared,
            &source.identity,
            &capture,
        )?;
        self.assert_current_observation(&input.observation)
            .map_err(exchange_unresolved)?;
        self.assert_active_policy_binding_current()
            .map_err(exchange_unresolved)?;
        render
            .assert_render_source_current(
                &actor,
                &request,
                &definition_digest,
                &authority_binding,
                &control_limits,
                input,
                context_ref,
                &source.identity,
            )
            .map_err(exchange_unresolved)?;
        Ok(decision)
    }
}

/// Admit the exact assembled provider bytes against the selected whole-input bound.
///
/// This is the served caller of the prepared-input budget: the bytes exist by now, so the only
/// admissible outcome is a refusal, and the refusal happens before `decide_prepared_for` can write
/// anything or spend a provider call. `None` publishes no separate reserve, which is the
/// pre-existing contract rather than an unlimited claim: the renderer still bounds these bytes by
/// the selected `max_context_bytes`, and response capacity stays bounded independently by the
/// provider configuration.
fn admit_assembled_managed_input(
    limits: &crate::ContextRenderLimits,
    input_bytes: usize,
) -> Result<(), ManagementError> {
    let Some(output_reserve_bytes) = limits.output_reserve_bytes else {
        return Ok(());
    };
    let bound = crate::context_memory::AssembledInputBound::new(
        limits.max_context_bytes,
        output_reserve_bytes,
    )
    .map_err(|error| {
        ManagementError::capability("context_whole_input_budget_invalid", error.to_string())
    })?;
    bound.admit(input_bytes).map_err(|error| {
        ManagementError::capability("context_whole_input_budget_exceeded", error.to_string())
    })?;
    Ok(())
}
