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
        let decision = self
            .provider_mut()?
            .decide_prepared_for(input, decision_profile_ref, context_ref, &prepared)
            .map_err(provider_error)?;
        self.assert_current_observation(&input.observation)?;
        self.assert_active_policy_binding_current()?;
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
        Ok(decision)
    }
}
