// SPDX-License-Identifier: MIT

use sts2_harness::context_control::PreparedContext;
use sts2_harness::exo_admission::ExoRuntimeAdmission;
use sts2_harness::management::{
    AdmittedInferenceProfileBinding, ContextRenderSource, LiveProviderSessionAdmission,
    ManagementError,
};
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, ExoConfig, ExoDecisionSource, ExoProvider, ExoSession,
    PolicyError,
};

use super::{LIVE_DECISION_ADAPTER, LIVE_DECISION_PROFILE_ID, runtime_v3_admission};

pub(super) struct ProfiledExoAdmission {
    pub(super) process: sts2_harness::ExoProcessConfig,
    pub(super) config: ExoConfig,
    pub(super) admission: ExoRuntimeAdmission,
    pub(super) profiles: Vec<AdmittedInferenceProfileBinding>,
}

impl LiveProviderSessionAdmission for ProfiledExoAdmission {
    fn open(self: Box<Self>) -> Result<Box<dyn DecisionSource + Send>, ManagementError> {
        let transport = runtime_v3_admission::admit(&self.admission, self.process)
            .map_err(|error| ManagementError::unavailable("provider_admission", error))?;
        let source =
            ExoDecisionSource::new(ExoSession::new(ExoProvider::new(transport, self.config)));
        Ok(Box::new(ProfileBoundDecisionSource {
            source: Box::new(source),
            profiles: self.profiles,
        }))
    }
}

struct ProfileBoundDecisionSource {
    source: Box<dyn DecisionSource + Send>,
    profiles: Vec<AdmittedInferenceProfileBinding>,
}

impl ProfileBoundDecisionSource {
    fn accepts(&self, profile_ref: &str, context_ref: &str) -> bool {
        self.profiles.iter().any(|admitted| {
            let binding = &admitted.binding;
            binding.profile_ref == profile_ref
                && binding.node_kind == "decide"
                && binding.profile_id == LIVE_DECISION_PROFILE_ID
                && binding.adapter == LIVE_DECISION_ADAPTER
                && binding.version == admitted.descriptor.version
                && binding.digest == admitted.descriptor.digest
                && binding.requested_model == admitted.descriptor.requested_model
                && binding.resolved_model == admitted.descriptor.resolved_model
                && admitted
                    .descriptor
                    .supports(LIVE_DECISION_PROFILE_ID, "decide")
                && admitted
                    .descriptor
                    .context_compatibility
                    .iter()
                    .any(|supported| supported == context_ref)
        })
    }
}

impl DecisionSource for ProfileBoundDecisionSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        Err(PolicyError::InputBlocked)
    }

    fn decide_with_game_information(
        &mut self,
        _input: &DecisionInput,
        _runtime: &mut dyn sts2_harness::EpisodeRuntimePort,
    ) -> Result<Decision, PolicyError> {
        Err(PolicyError::InputBlocked)
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<Decision, PolicyError> {
        if !self.accepts(decision_profile_ref, context_ref) {
            return Err(PolicyError::InputBlocked);
        }
        self.source
            .decide_for(input, decision_profile_ref, context_ref)
    }

    fn prepare_managed_context(
        &mut self,
        input: &DecisionInput,
        source: &ContextRenderSource,
    ) -> Result<PreparedContext, PolicyError> {
        self.source.prepare_managed_context(input, source)
    }

    fn managed_render_config(&self) -> Option<ExoConfig> {
        self.source.managed_render_config()
    }

    fn decide_prepared_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        prepared: &PreparedContext,
    ) -> Result<Decision, PolicyError> {
        if !self.accepts(decision_profile_ref, context_ref) {
            return Err(PolicyError::InputBlocked);
        }
        self.source
            .decide_prepared_for(input, decision_profile_ref, context_ref, prepared)
    }

    fn action_completed(&mut self, settled: bool) {
        self.source.action_completed(settled);
    }

    fn model_execution_id(&self) -> Option<sts2_harness::ModelExecutionId> {
        self.source.model_execution_id()
    }

    fn close(&mut self) -> Result<(), PolicyError> {
        self.source.close()
    }
}

#[cfg(test)]
#[path = "workflow_service_profile_dispatch_source_tests.rs"]
mod tests;
