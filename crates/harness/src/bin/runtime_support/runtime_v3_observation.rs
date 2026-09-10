// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    /// Read the authoritative Runtime-v3 projection and, for expert profiles, compose it with
    /// the expert projection. Idle-transition waits use the bounded lagging-expert retry because
    /// the normal read can legitimately win a cross-process race with the expert read.
    pub(super) fn observe_for_idle_transition(
        &mut self,
    ) -> Result<EpisodeObservation, sts2_harness::PortError> {
        self.observe_inner(true)
    }

    fn observe_inner(
        &mut self,
        allow_lagging_expert_retry: bool,
    ) -> Result<EpisodeObservation, sts2_harness::PortError> {
        let arguments = self.context(self.generation);
        let value = self
            .call_tool("sts2.observe", arguments)
            .map_err(|error| wire::port_error("observe_failed", error, false))?;
        let parsed = parse::observation(&value, "state_response", &self.config)
            .map_err(|error| wire::port_error("observe_invalid", error, false))?;
        let baseline = self.install(parsed);
        let observation = if self.is_expert_profile() {
            let composed = if allow_lagging_expert_retry {
                self.compose_current_observation_for_idle(baseline)
            } else {
                self.compose_current_observation(baseline)
            };
            composed.map_err(|error| wire::port_error("expert_observe_invalid", error, false))?
        } else {
            baseline
        };
        let _ = self
            .telemetry
            .observation(ObservationSource::Observe, &observation);
        Ok(observation)
    }
}
