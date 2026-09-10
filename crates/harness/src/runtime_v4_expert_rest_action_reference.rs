// SPDX-License-Identifier: MIT

fn validate_action_reference(value: &Value) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let Some(reference) = value.as_object() else {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidShape);
    };
    exact_fields(reference, &["action_id", "action"])?;
    if !identity(&reference["action_id"]) {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let Some(action) = reference["action"].as_object() else {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidShape);
    };
    let kind = action["kind"]
        .as_str()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    match kind {
        "rest_option" => {
            exact_fields(action, &["kind", "rest_option_id"])?;
            if !matches!(
                action["rest_option_id"].as_str(),
                Some(
                    "clone"
                        | "cook"
                        | "dig"
                        | "hatch"
                        | "heal"
                        | "kindle"
                        | "lift"
                        | "smith"
                        | "mend"
                )
            ) {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        "select_card" => {
            exact_fields(
                action,
                &["kind", "selection_id", "rest_option_id", "card_id"],
            )?;
            if action["rest_option_id"] != "smith"
                || !identity(&action["selection_id"])
                || !identity(&action["card_id"])
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        "select_player" => {
            exact_fields(
                action,
                &["kind", "selection_id", "rest_option_id", "player_id"],
            )?;
            if action["rest_option_id"] != "mend"
                || !identity(&action["selection_id"])
                || !identity(&action["player_id"])
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        "confirm_selection" | "cancel_selection" => {
            exact_fields(action, &["kind", "selection_id", "rest_option_id"])?;
            if !matches!(action["rest_option_id"].as_str(), Some("smith" | "mend"))
                || !identity(&action["selection_id"])
            {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
    }
    Ok(())
}

fn validate_selector(
    value: &Value,
    expert: &crate::RuntimeV4ExpertObservation,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let selector = value
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    exact_fields(
        selector,
        &[
            "selection_id",
            "selection_kind",
            "required_count",
            "selected_choice_ids",
            "remaining_count",
            "legal_actions",
        ],
    )?;
    if !identity(&selector["selection_id"])
        || !matches!(selector["selection_kind"].as_str(), Some("card" | "player"))
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let required = positive_count(&selector["required_count"])?;
    let selected = bounded_unique_ids(&selector["selected_choice_ids"])?;
    let remaining = bounded_count(&selector["remaining_count"])?;
    if selected.len() > required || remaining != required - selected.len() {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let option = selector_option_from_actions(&selector["legal_actions"])?;
    if (selector["selection_kind"] == "card" && option != "smith")
        || (selector["selection_kind"] == "player" && option != "mend")
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let choice_values = expert.as_value()["state"]["choices"]
        .as_array()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    let mut choices = BTreeSet::new();
    for choice in choice_values {
        let choice_id = choice["choice_id"]
            .as_str()
            .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
        if !identity(&Value::String(choice_id.to_owned())) || !choices.insert(choice_id) {
            return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
        }
    }
    if choices.is_empty() {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    if selected.iter().any(|choice_id| !choices.contains(choice_id)) {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let actions = selector["legal_actions"]
        .as_array()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    if actions.is_empty() || actions.len() > MAX_SELECTOR_ITEMS {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let mut ids = BTreeSet::new();
    let mut has_choice = false;
    let mut has_confirm = false;
    let mut has_cancel = false;
    for reference in actions {
        validate_action_reference(reference)?;
        let id = reference["action_id"]
            .as_str()
            .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
        if !ids.insert(id)
            || reference["action"]["selection_id"] != selector["selection_id"]
            || reference["action"]["rest_option_id"] != option
        {
            return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
        }
        match reference["action"]["kind"]
            .as_str()
            .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?
        {
            "confirm_selection" => has_confirm = true,
            "cancel_selection" => has_cancel = true,
            "select_card" => {
                let card_id = reference["action"]["card_id"]
                    .as_str()
                    .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
                if selector["selection_kind"] != "card"
                    || !choices.contains(card_id)
                {
                    return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
                }
                has_choice = true;
            }
            "select_player" => {
                let player_id = reference["action"]["player_id"]
                    .as_str()
                    .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
                if selector["selection_kind"] != "player"
                    || !choices.contains(player_id)
                {
                    return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
                }
                has_choice = true;
            }
            _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
        }
    }
    if !has_cancel
        || remaining == 0 && !has_confirm
        || remaining > 0 && has_confirm
        || remaining > 0 && !has_choice
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}
