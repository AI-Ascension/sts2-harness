// SPDX-License-Identifier: MIT

fn compose_with_rest_selector(
    baseline: &EpisodeObservation,
    selector_actions: EpisodeLegalActionSet,
    selector_payloads: BTreeMap<String, Value>,
    expert: &RuntimeV4ExpertObservation,
) -> Result<ComposedExpertObservation, String> {
    if baseline.state_id() != expert.state_id() || baseline.generation() != expert.generation() {
        return Err(String::from(
            "Runtime-v4 expert state does not match the Runtime-v3 observation",
        ));
    }
    selector_actions
        .assert_matches(expert.state_id(), expert.generation())
        .map_err(|error| format!("REST selector is stale for expert state: {error}"))?;
    let mut composed = expert_only_observation(expert)?;
    if composed.observation.stage() != baseline.stage() {
        return Err(String::from(
            "Runtime-v4 expert stage does not match the Runtime-v3 observation",
        ));
    }
    composed.actions = selector_actions;
    composed.payloads = selector_payloads;
    Ok(composed)
}
