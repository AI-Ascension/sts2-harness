// SPDX-License-Identifier: MIT

fn parse_strict(bytes: &[u8]) -> Result<Value, RuntimeV4ExpertRestActionParseError> {
    if bytes.len() > MAX_ACTION_BYTES {
        return Err(RuntimeV4ExpertRestActionParseError::TooLarge);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let StrictValue(value) = StrictValue::deserialize(&mut deserializer)
        .map_err(|_| RuntimeV4ExpertRestActionParseError::MalformedJson)?;
    deserializer
        .end()
        .map_err(|_| RuntimeV4ExpertRestActionParseError::MalformedJson)?;
    Ok(value)
}

fn validate_root(value: &Value, kind: &str) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let Some(root) = value.as_object() else {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidShape);
    };
    if root.len() != ROOT_FIELDS.len() || ROOT_FIELDS.iter().any(|field| !root.contains_key(*field))
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidShape);
    }
    if value["protocol_version"] != RUNTIME_V4_EXPERT_REST_ACTION_PROTOCOL_VERSION
        || value["schema_digest"] != RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_DIGEST
        || value["profile"] != "expert-rest-action"
        || value["kind"] != kind
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let Some(provenance) = value["provenance"].as_object() else {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidShape);
    };
    if provenance.len() != 3
        || provenance["artifact"] != RUNTIME_V4_EXPERT_REST_ACTION_ARTIFACT
        || provenance["source"] != RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_SOURCE
        || provenance["generator"] != RUNTIME_V4_EXPERT_REST_ACTION_GENERATOR
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    for field in [
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "state_id",
        "operation_id",
    ] {
        if !identity(&value[field]) {
            return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
        }
    }
    for field in ["lease_epoch", "generation"] {
        if value[field]
            .as_u64()
            .is_none_or(|number| number > MAX_SAFE_INTEGER)
        {
            return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
        }
    }
    Ok(())
}

fn validate_request(value: &Value) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    if !value["status"].is_null()
        || !value["observation"].is_null()
        || !value["transition"].is_null()
        || !value["effect_witness"].is_null()
        || !value["error_code"].is_null()
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    validate_action_reference(&value["action"])
}

fn validate_response(
    value: &Value,
) -> Result<RuntimeV4ExpertRestActionStatus, RuntimeV4ExpertRestActionParseError> {
    let status = value["status"]
        .as_str()
        .and_then(RuntimeV4ExpertRestActionStatus::parse)
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    match status {
        RuntimeV4ExpertRestActionStatus::Accepted => {
            validate_action_reference(&value["action"])?;
            require_null(&value["observation"])?;
            require_null(&value["transition"])?;
            require_null(&value["effect_witness"])?;
            require_null(&value["error_code"])?;
        }
        RuntimeV4ExpertRestActionStatus::Rejected
        | RuntimeV4ExpertRestActionStatus::Unknown
        | RuntimeV4ExpertRestActionStatus::Cancelled => {
            validate_action_reference(&value["action"])?;
            require_null(&value["observation"])?;
            require_null(&value["transition"])?;
            require_null(&value["effect_witness"])?;
            if !identity(&value["error_code"]) {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
        }
        RuntimeV4ExpertRestActionStatus::Settled => validate_settled(value)?,
    }
    Ok(status)
}

fn validate_settled(value: &Value) -> Result<(), RuntimeV4ExpertRestActionParseError> {
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
            if (option == "smith"
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
