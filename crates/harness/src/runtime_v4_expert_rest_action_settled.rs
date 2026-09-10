// SPDX-License-Identifier: MIT

fn validate_settled(value: &Value) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    require_null(&value["error_code"])?;
    validate_action_reference(&value["action"])?;
    let observation = value["observation"].clone();
    let expert = crate::RuntimeV4ExpertObservation::from_value(observation)
        .map_err(|_| RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    if expert.state_id() != value["state_id"].as_str().unwrap_or("")
        || expert.generation() != value["generation"].as_u64().unwrap_or(0)
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let transition = value["transition"]
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    let transition_kind = transition["kind"]
        .as_str()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    let before = number(&transition["before_generation"])?;
    let after = number(&transition["after_generation"])?;
    if after <= before
        || after != value["generation"].as_u64().unwrap_or(0)
        || transition["rest_option_id"] != value["action"]["action"]["rest_option_id"]
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    match transition_kind {
        "rest_option_completed" => {
            exact_fields(
                transition,
                &[
                    "kind",
                    "before_generation",
                    "after_generation",
                    "rest_option_id",
                    "completed",
                    "effect_witness",
                ],
            )?;
            if transition["completed"] != true
                || value["action"]["action"]["kind"] != "rest_option"
                || !matches!(
                    transition["rest_option_id"].as_str(),
                    Some("clone" | "cook" | "dig" | "hatch" | "heal" | "kindle" | "lift")
                )
                || value["effect_witness"].is_null()
                || transition["effect_witness"] != value["effect_witness"]
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            validate_witness(
                &value["effect_witness"],
                value,
                transition["rest_option_id"].as_str().unwrap_or(""),
                false,
            )?;
        }
        "rest_option_selection_requested" => {
            exact_fields(
                transition,
                &[
                    "kind",
                    "before_generation",
                    "after_generation",
                    "rest_option_id",
                    "selector",
                    "effect_witness",
                ],
            )?;
            if value["action"]["action"]["kind"] != "rest_option"
                || !value["effect_witness"].is_null()
                || !transition["effect_witness"].is_null()
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            validate_selector(&transition["selector"], &expert)?;
        }
        "rest_option_selection_progressed" => {
            exact_fields(
                transition,
                &[
                    "kind",
                    "before_generation",
                    "after_generation",
                    "rest_option_id",
                    "selection_id",
                    "selection_kind",
                    "required_count",
                    "selected_choice_ids",
                    "remaining_count",
                    "selector",
                    "effect_witness",
                ],
            )?;
            if !matches!(
                value["action"]["action"]["kind"].as_str(),
                Some("select_card" | "select_player")
            ) || !value["effect_witness"].is_null()
                || !transition["effect_witness"].is_null()
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            validate_selection_fields(transition)?;
            validate_selector(&transition["selector"], &expert)?;
            if transition["selector"]["selection_id"] != transition["selection_id"]
                || transition["selector"]["selection_kind"] != transition["selection_kind"]
                || transition["selector"]["required_count"] != transition["required_count"]
                || transition["selector"]["selected_choice_ids"]
                    != transition["selected_choice_ids"]
                || transition["selector"]["remaining_count"] != transition["remaining_count"]
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            let action = &value["action"]["action"];
            let selected = transition["selected_choice_ids"]
                .as_array()
                .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
            let action_choice = match action["kind"].as_str() {
                Some("select_card") => action["card_id"].as_str(),
                Some("select_player") => action["player_id"].as_str(),
                _ => None,
            }
            .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
            if action["selection_id"] != transition["selection_id"]
                || !selected.iter().any(|choice| choice.as_str() == Some(action_choice))
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        "rest_option_selection_completed" => {
            exact_fields(
                transition,
                &[
                    "kind",
                    "before_generation",
                    "after_generation",
                    "rest_option_id",
                    "selection_id",
                    "selection_kind",
                    "required_count",
                    "selected_choice_ids",
                    "remaining_count",
                    "completed",
                    "effect_witness",
                ],
            )?;
            if value["effect_witness"].is_null()
                || transition["effect_witness"] != value["effect_witness"]
                || transition["completed"] != true
                || transition["remaining_count"] != 0
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            validate_selection_fields(transition)?;
            if transition["selected_choice_ids"]
                .as_array()
                .is_none_or(|ids| {
                    ids.len() != transition["required_count"].as_u64().unwrap_or(0) as usize
                })
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            let action_kind = value["action"]["action"]["kind"].as_str().unwrap_or("");
            let option = transition["rest_option_id"].as_str().unwrap_or("");
            if !matches!(option, "smith" | "mend")
                || (option == "smith"
                && (transition["selection_kind"] != "card" || action_kind != "confirm_selection"))
                || (option == "mend" && transition["selection_kind"] != "player")
                || !matches!(action_kind, "confirm_selection" | "select_player")
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            if value["action"]["action"]["selection_id"] != transition["selection_id"]
                || (option == "mend"
                    && action_kind == "select_player"
                    && value["action"]["action"]["player_id"]
                        != transition["effect_witness"]["target_player_id"])
            {
                return Err(RuntimeV4ExpertRestActionParseError::IdentityMismatch);
            }
            validate_witness(&value["effect_witness"], value, option, true)?;
            let witness = &value["effect_witness"];
            if option == "smith"
                && (witness["kind"] != "smith_applied"
                    || witness["evidence"]["kind"] != "card_change"
                    || witness["evidence"]["upgraded_card_ids"]
                        != transition["selected_choice_ids"])
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            if option == "mend"
                && (witness["kind"] != "mend_applied"
                    || transition["selected_choice_ids"]
                        .as_array()
                        .and_then(|ids| ids.first())
                        != witness["target_player_id"]
                            .as_str()
                            .map(Value::from)
                            .as_ref())
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
    }
    Ok(())
}
