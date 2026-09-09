// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    pub(super) fn compose_current_observation(
        &mut self,
        baseline: EpisodeObservation,
    ) -> Result<EpisodeObservation, String> {
        self.compose_current_observation_classified(baseline, false)
            .map_err(|error| error.message().to_owned())
    }

    /// Compose a fresh ordinary reobserve with the expert projection. The expert state call is
    /// a bounded recovery read here, while parsing and cross-projection identity remain terminal.
    pub(super) fn compose_current_observation_recovery(
        &mut self,
        baseline: EpisodeObservation,
    ) -> Result<EpisodeObservation, RuntimeV3ToolError> {
        self.compose_current_observation_classified(baseline, true)
    }

    fn compose_current_observation_classified(
        &mut self,
        baseline: EpisodeObservation,
        catalog_read: bool,
    ) -> Result<EpisodeObservation, RuntimeV3ToolError> {
        let expert = self.expert_state_classified(catalog_read)?;
        let normal_actions = self
            .current_actions
            .clone()
            .ok_or_else(|| {
                RuntimeV3ToolError::Terminal(String::from(
                    "normal catalog is unavailable for expert composition",
                ))
        })?;
        let normal_payloads = self.payloads.clone();
        let composed = if self.is_rest_profile()
            && self.rest_selector_actions.as_ref().is_some_and(|selector| {
                selector
                    .assert_matches(expert.state_id(), expert.generation())
                    .is_ok()
            })
        {
            compose_with_rest_selector(
                &baseline,
                self.rest_selector_actions.clone().ok_or_else(|| {
                    RuntimeV3ToolError::Terminal(String::from(
                        "REST selector disappeared during expert composition",
                    ))
                })?,
                self.rest_selector_payloads.clone(),
                &expert,
            )
            .map_err(RuntimeV3ToolError::Terminal)?
        } else {
            compose_with_normal(&baseline, &normal_actions, &normal_payloads, &expert)
                .map_err(RuntimeV3ToolError::Terminal)?
        };
        let mut composed = composed;
        if self.is_rest_profile() {
            self.overlay_active_rest_selector(&mut composed)
                .map_err(RuntimeV3ToolError::Terminal)?;
        }
        self.install_composed(&composed);
        Ok(composed.observation)
    }

}
