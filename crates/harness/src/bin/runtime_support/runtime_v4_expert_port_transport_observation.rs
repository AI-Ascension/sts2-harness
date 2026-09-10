// SPDX-License-Identifier: MIT

use super::parse;

// Normal and expert observations are read through separate MCP processes. A host transition can
// therefore advance the authoritative generation between those reads. Give that forward-only
// race a small, explicit reobserve budget; identity regressions and malformed content still fail
// closed at the composition boundary.
const MAX_EXPERT_COMPOSITION_REOBSERVES: usize = 2;

impl RuntimeV3Port {
    pub(super) fn compose_current_observation(
        &mut self,
        baseline: EpisodeObservation,
    ) -> Result<EpisodeObservation, String> {
        self.compose_current_observation_classified(baseline, false, false)
            .map_err(|error| error.message().to_owned())
    }

    /// Compose an idle-transition observation with a bounded retry when the expert process
    /// returns a lagging projection. The normal read already advanced, so rereading the expert
    /// projection is sufficient; a persistent lag remains terminal.
    pub(super) fn compose_current_observation_for_idle(
        &mut self,
        baseline: EpisodeObservation,
    ) -> Result<EpisodeObservation, String> {
        self.compose_current_observation_classified(baseline, false, true)
            .map_err(|error| error.message().to_owned())
    }

    /// Compose a fresh ordinary reobserve with the expert projection. The expert state call is
    /// a bounded recovery read here, while parsing and cross-projection identity remain terminal.
    pub(super) fn compose_current_observation_recovery(
        &mut self,
        baseline: EpisodeObservation,
    ) -> Result<EpisodeObservation, RuntimeV3ToolError> {
        self.compose_current_observation_classified(baseline, true, false)
    }

    fn compose_current_observation_classified(
        &mut self,
        mut baseline: EpisodeObservation,
        catalog_read: bool,
        allow_lagging_expert_retry: bool,
    ) -> Result<EpisodeObservation, RuntimeV3ToolError> {
        for reobserve_count in 0..=MAX_EXPERT_COMPOSITION_REOBSERVES {
            let expert = self.expert_state_classified(catalog_read)?;
            let normal_actions = self
                .current_actions
                .clone()
                .ok_or_else(|| {
                    RuntimeV3ToolError::Terminal(String::from(
                        "normal catalog is unavailable for expert composition",
                    ))
                })?;

            // A newer expert generation means the normal snapshot became stale while the two
            // projections were being read. Refresh the normal projection and try the expert
            // read again. Equal-generation identity changes and retrograde generations bypass
            // this path and remain terminal through the strict composition checks below.
            if expert.generation() > baseline.generation() {
                if reobserve_count == MAX_EXPERT_COMPOSITION_REOBSERVES {
                    return Err(RuntimeV3ToolError::Terminal(String::from(
                        "Runtime-v4 expert state advanced during bounded Runtime-v3 reobserve",
                    )));
                }
                baseline = self.reobserve_for_composition(&baseline)?;
                continue;
            }
            if expert.generation() < baseline.generation() && allow_lagging_expert_retry {
                if reobserve_count == MAX_EXPERT_COMPOSITION_REOBSERVES {
                    return Err(RuntimeV3ToolError::Terminal(String::from(
                        "Runtime-v4 expert state remained behind the Runtime-v3 idle observation",
                    )));
                }
                // The normal projection is newer than this expert read. Re-read only the
                // expert projection so the idle wait cannot repeat or manufacture a mutation.
                continue;
            }

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
            return Ok(composed.observation);
        }

        Err(RuntimeV3ToolError::Terminal(String::from(
            "Runtime-v4 expert composition did not produce an observation",
        )))
    }

    fn reobserve_for_composition(
        &mut self,
        baseline: &EpisodeObservation,
    ) -> Result<EpisodeObservation, RuntimeV3ToolError> {
        let value = self.call_tool_classified(
            "sts2.reobserve",
            self.context(baseline.generation()),
        )?;
        let parsed = parse::observation(&value, "reobserve_response", &self.config).map_err(|error| {
            RuntimeV3ToolError::Terminal(format!(
                "Runtime-v3 reobserve for expert composition is invalid: {error}"
            ))
        })?;
        if parsed.observation.generation() < baseline.generation()
            || (parsed.observation.generation() == baseline.generation()
                && parsed.observation.state_id() != baseline.state_id())
        {
            return Err(RuntimeV3ToolError::Terminal(String::from(
                "Runtime-v3 reobserve regressed during expert composition",
            )));
        }
        Ok(self.install(parsed))
    }

}
