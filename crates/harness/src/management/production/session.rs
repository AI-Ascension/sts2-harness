// SPDX-License-Identifier: MIT

use super::*;

#[path = "session_errors.rs"]
mod errors;
#[path = "session_inference_profile.rs"]
pub(super) mod inference_profile;
#[path = "session_managed.rs"]
mod managed;
#[path = "session_policy.rs"]
mod policy;

pub(super) use errors::{
    decision_provider_error, exchange_unresolved, provider_error, runtime_error,
};

impl ProductionLiveWorkflowSession {
    fn refresh_authority_lease_binding(&mut self) -> Result<(), ManagementError> {
        let lease = self
            .runtime
            .current_lease_binding()
            .map_err(runtime_error("runtime_lease_binding_unavailable"))?;
        if lease.instance_id != self.authority_binding.instance_id
            || lease.session_id != self.authority_binding.session_id
            || lease.run_id != self.authority_binding.run_id
            || lease.lease_id.is_empty()
            || lease.lease_epoch == 0
        {
            return Err(ManagementError::conflict(
                "runtime_lease_binding_mismatch",
                "post-launch gateway lease does not match the admitted runtime scope",
            ));
        }
        self.authority_binding.lease_id = lease.lease_id;
        self.authority_binding.lease_epoch = lease.lease_epoch;
        super::runtime_authority::validate_runtime_authority_binding(
            &self.request,
            &self.definition_digest,
            &self.authority_binding,
        )
    }

    /// The host cannot be atomically locked across a provider request. A
    /// fresh read therefore fences the provider call, and the runtime repeats
    /// the read at dispatch while the gateway/MCP action envelope carries its
    /// authoritative lease and generation fence.
    pub(super) fn assert_current_observation(
        &mut self,
        expected: &EpisodeObservation,
    ) -> Result<(), ManagementError> {
        let current = self
            .runtime
            .observe()
            .map_err(runtime_error("live_provider_fence_failed"))?;
        self.record_context_observation(&current)?;
        if current.state_id() != expected.state_id()
            || current.generation() != expected.generation()
        {
            return Err(ManagementError::conflict(
                "live_provider_generation_stale",
                "the gateway/MCP observation changed before provider inference; re-observe is required",
            ));
        }
        Ok(())
    }

    pub(super) fn provider_mut(
        &mut self,
    ) -> Result<&mut (dyn DecisionSource + Send + 'static), ManagementError> {
        self.provider.as_deref_mut().ok_or_else(|| {
            ManagementError::unavailable(
                "provider_session_not_open",
                "provider construction must follow the authoritative launch observation",
            )
        })
    }

    pub(super) fn record_context_observation(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), ManagementError> {
        if let Some(owner) = &self.context_observations {
            owner.record_observation(
                &self.actor,
                &self.request,
                &self.definition_digest,
                &self.authority_binding,
                observation,
                self.context_control_limits.as_ref().ok_or_else(|| {
                    ManagementError::capability(
                        "selected_context_control_limits_required",
                        "served context observation has no admitted control limits",
                    )
                })?,
            )?;
        }
        Ok(())
    }

    pub(super) fn record_context_legal_actions(
        &self,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), ManagementError> {
        if let Some(owner) = &self.context_observations {
            owner.record_legal_actions(
                &self.actor,
                &self.request,
                &self.definition_digest,
                &self.authority_binding,
                actions,
            )?;
        }
        Ok(())
    }

    pub(super) fn invalidate_context_observation(&self) {
        if let Some(owner) = &self.context_observations {
            owner.invalidate(&self.actor, &self.request, &self.definition_digest);
        }
    }
}

impl LiveWorkflowSession for ProductionLiveWorkflowSession {
    fn launch(&mut self) -> Result<(), ManagementError> {
        self.runtime
            .launch()
            .map_err(runtime_error("live_launch_failed"))?;
        if self.context_observations.is_some() {
            self.refresh_authority_lease_binding()?;
        }
        let observation = self
            .runtime
            .observe()
            .map_err(runtime_error("live_launch_fence_failed"))?;
        self.record_context_observation(&observation)?;
        let actions = self
            .runtime
            .legal_actions(observation.state_id(), observation.generation())
            .map_err(runtime_error("live_launch_catalog_failed"))?;
        self.record_context_legal_actions(&actions)?;
        self.launch_observation = Some(observation);
        let workflow_run_id =
            crate::management::live_run_id(&self.request, &self.definition_digest)?;
        let active_policy = self.provider_policy.load_active_policy(
            &self.actor,
            &self.request,
            &workflow_run_id,
            &self.definition,
            &self.provider_capabilities,
        )?;
        self.active_policy_binding = Some((
            active_policy.policy_sha256,
            active_policy.adoption_generation,
        ));
        self.provider = Some(self.provider_factory.open_provider(
            &self.request,
            &self.actor,
            &self.definition,
            &self.definition_digest,
        )?);
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError> {
        if let Some(observation) = self.launch_observation.take() {
            return Ok(observation);
        }
        let observation = self
            .runtime
            .observe()
            .map_err(runtime_error("live_observe_failed"))?;
        self.record_context_observation(&observation)?;
        Ok(observation)
    }

    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, ManagementError> {
        let observation = self
            .runtime
            .observe_projection(projection_ref)
            .map_err(runtime_error("live_observe_failed"))?;
        self.record_context_observation(&observation)?;
        let actions = self
            .runtime
            .legal_actions(observation.state_id(), observation.generation())
            .map_err(runtime_error("live_catalog_failed"))?;
        self.record_context_legal_actions(&actions)?;
        Ok(observation)
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, ManagementError> {
        let actions = self
            .runtime
            .legal_actions(state_id, generation)
            .map_err(runtime_error("live_catalog_failed"))?;
        self.record_context_legal_actions(&actions)?;
        Ok(actions)
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<crate::Decision, ManagementError> {
        self.assert_current_observation(&input.observation)?;
        self.admit_active_policy_binding()?;
        let decision = self
            .provider_mut()?
            .decide(input)
            .map_err(decision_provider_error)?;
        self.assert_active_policy_binding_current()
            .map_err(exchange_unresolved)?;
        Ok(decision)
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<crate::Decision, ManagementError> {
        self.assert_current_observation(&input.observation)?;
        self.admit_active_policy_binding()?;
        self.admit_inference_profile_binding(decision_profile_ref)?;
        if let Some(render) = self
            .context_render
            .as_ref()
            .filter(|render| render.render_required())
            .cloned()
        {
            return self.decide_with_managed_context(
                input,
                decision_profile_ref,
                context_ref,
                render,
            );
        }
        let decision = self
            .provider_mut()?
            .decide_for(input, decision_profile_ref, context_ref)
            .map_err(decision_provider_error)?;
        self.assert_active_policy_binding_current()
            .map_err(exchange_unresolved)?;
        Ok(decision)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError> {
        let observation = self
            .runtime
            .observe()
            .map_err(runtime_error("live_action_fence_failed"))?;
        self.record_context_observation(&observation)?;
        if observation.state_id() != identity.state_id
            || observation.generation() != identity.generation
        {
            return Err(ManagementError::conflict(
                "live_action_generation_stale",
                "the gateway/MCP observation changed before action dispatch; re-observe is required",
            ));
        }
        self.runtime
            .dispatch_action(identity, action)
            .map_err(runtime_error("live_dispatch_failed"))
    }

    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, ManagementError> {
        let result = self
            .runtime
            .wait_for_transition(operation_id, wait_for_millis)
            .map_err(|error| {
                ManagementError::unresolved("live_transition_wait_failed", error.to_string())
            });
        if let Ok(sample) = &result
            && let Some(observation) = sample.observation()
        {
            self.record_context_observation(observation)?;
        }
        result
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, ManagementError> {
        self.runtime.reconcile(operation_id).map_err(|error| {
            ManagementError::unresolved("live_reconcile_failed", error.to_string())
        })
    }

    fn release_lease(&mut self) -> Result<(), ManagementError> {
        self.invalidate_context_observation();
        RecoveryPort::release_lease(&mut *self.runtime)
            .map_err(|error| ManagementError::unavailable("live_release_failed", error.to_string()))
    }

    fn stop_episode(&mut self) -> Result<(), ManagementError> {
        self.invalidate_context_observation();
        RecoveryPort::stop_episode(&mut *self.runtime)
            .map_err(|error| ManagementError::unavailable("live_stop_failed", error.to_string()))
    }

    fn action_completed(&mut self, settled: bool) {
        if let Some(provider) = self.provider.as_mut() {
            provider.action_completed(settled);
        }
    }

    fn model_execution_id(&self) -> Option<crate::ModelExecutionId> {
        self.provider
            .as_ref()
            .and_then(|provider| provider.model_execution_id())
    }
}
