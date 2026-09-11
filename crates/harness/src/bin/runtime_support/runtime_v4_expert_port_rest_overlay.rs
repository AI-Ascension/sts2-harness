// SPDX-License-Identifier: MIT

fn overlay_rest_selector(
    composed: &mut ComposedExpertObservation,
    selector: &Value,
) -> Result<(), String> {
    let legal_actions = selector
        .get("legal_actions")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("REST selector omitted legal actions"))?
        .iter()
        .map(selector_provider_action)
        .collect::<Result<Vec<_>, _>>()?;
    let mut fair_play = composed.observation.fair_play().as_value().clone();
    fair_play["legal_actions"] = Value::Array(legal_actions);
    fair_play["harness_projection"] = Value::String(
        sts2_harness::RUNTIME_V4_EXPERT_FAIR_PLAY_PROJECTION.to_owned(),
    );
    composed.observation = EpisodeObservation::new(
        composed.observation.state_id(),
        composed.observation.generation(),
        composed.observation.stage(),
        composed.observation.actionable(),
        composed.observation.modal_blocking(),
        composed.observation.input_enabled(),
        fair_play,
    )
    .map_err(|error| format!("REST selector observation is invalid: {error}"))?;
    Ok(())
}

fn selector_provider_action(value: &Value) -> Result<Value, String> {
    let action_id = value["action_id"]
        .as_str()
        .ok_or_else(|| String::from("REST selector action ID is invalid"))?;
    let action = value
        .get("action")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("REST selector action payload is missing"))?;
    let kind = action["kind"]
        .as_str()
        .ok_or_else(|| String::from("REST selector action kind is invalid"))?;
    let projected = match kind {
        "select_card" => json!({
            "kind": "select_card",
            "card_id": action["card_id"].as_str().ok_or_else(|| {
                String::from("REST card selector omitted its card target")
            })?
        }),
        "select_player" => json!({
            "kind": "select_player",
            "player_id": action["player_id"].as_str().ok_or_else(|| {
                String::from("REST player selector omitted its player target")
            })?
        }),
        "confirm_selection" | "cancel_selection" => json!({"kind": kind}),
        _ => return Err(format!("REST selector action kind {kind} is unsupported")),
    };
    Ok(json!({"action_id": action_id, "action": projected}))
}

impl RuntimeV3Port {
    fn apply_active_rest_selector(
        &self,
        composed: &mut ComposedExpertObservation,
    ) -> Result<(), String> {
        let actions = self
            .rest_selector_actions
            .as_ref()
            .ok_or_else(|| String::from("REST selector was not retained"))?;
        actions
            .assert_matches(composed.observation.state_id(), composed.observation.generation())
            .map_err(|error| format!("REST selector is stale for composed observation: {error}"))?;
        composed.actions = actions.clone();
        composed.payloads = self.rest_selector_payloads.clone();
        let selector = self
            .rest_selector_value
            .as_ref()
            .ok_or_else(|| String::from("REST selector payload was not retained"))?;
        overlay_rest_selector(composed, selector)
    }

    fn overlay_active_rest_selector(
        &self,
        composed: &mut ComposedExpertObservation,
    ) -> Result<(), String> {
        let Some(actions) = self.rest_selector_actions.as_ref() else {
            return Ok(());
        };
        if actions
            .assert_matches(composed.observation.state_id(), composed.observation.generation())
            .is_err()
        {
            return Ok(());
        }
        self.apply_active_rest_selector(composed)
    }
}
