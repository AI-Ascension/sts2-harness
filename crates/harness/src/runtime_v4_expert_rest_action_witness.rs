// SPDX-License-Identifier: MIT

fn selector_option_from_actions(
    value: &Value,
) -> Result<&str, RuntimeV4ExpertRestActionParseError> {
    let actions = value
        .as_array()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    let first = actions
        .first()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    let option = first["action"]["rest_option_id"]
        .as_str()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    if !matches!(option, "smith" | "mend") {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(option)
}

fn validate_selection_fields(
    value: &Map<String, Value>,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    if !identity(&value["selection_id"])
        || !matches!(value["selection_kind"].as_str(), Some("card" | "player"))
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let required = positive_count(&value["required_count"])?;
    let selected = bounded_unique_ids(&value["selected_choice_ids"])?;
    let remaining = bounded_count(&value["remaining_count"])?;
    if selected.len() > required || remaining != required - selected.len() {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}

fn validate_witness(
    value: &Value,
    root: &Value,
    option: &str,
    selection: bool,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let witness = value
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    let required = if option == "mend" {
        [
            "version",
            "kind",
            "operation_id",
            "rest_option_id",
            "generation",
            "evidence",
            "target_player_id",
        ]
        .as_slice()
    } else {
        [
            "version",
            "kind",
            "operation_id",
            "rest_option_id",
            "generation",
            "evidence",
        ]
        .as_slice()
    };
    exact_fields(witness, required)?;
    if witness["version"] != "rest-effect-witness-v1"
        || witness["operation_id"] != root["operation_id"]
        || witness["rest_option_id"] != option
        || witness["generation"] != root["generation"]
        || !identity(&witness["operation_id"])
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let expected_kind = match option {
        "clone" => "clone_applied",
        "cook" => "cook_applied",
        "dig" => "dig_applied",
        "hatch" => "hatch_applied",
        "heal" => "heal_applied",
        "kindle" => "kindle_applied",
        "lift" => "lift_applied",
        "smith" => "smith_applied",
        "mend" => "mend_applied",
        _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
    };
    if witness["kind"] != expected_kind {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let evidence = witness
        .get("evidence")
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    let evidence_kind = evidence
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    match option {
        "heal" => {
            if evidence_kind != "hp_change" {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            validate_hp_evidence(evidence, true)?;
        }
        "mend" => match evidence_kind {
            "hp_change" => validate_hp_evidence(evidence, true)?,
            "native_completion" => {
                validate_native_evidence(evidence, root["state_id"].as_str().unwrap_or(""))?;
            }
            _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
        },
        "clone" | "cook" | "smith" => {
            validate_card_evidence(evidence, option)?;
        }
        "dig" | "hatch" => {
            validate_relic_evidence(evidence)?;
        }
        "lift" => match evidence_kind {
            "stat_change" => validate_stat_evidence(evidence)?,
            "native_completion" => {
                validate_native_evidence(evidence, root["state_id"].as_str().unwrap_or(""))?;
            }
            _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
        },
        "kindle" => {
            if evidence_kind != "native_completion" {
                return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
            }
            validate_native_evidence(evidence, root["state_id"].as_str().unwrap_or(""))?;
        }
        _ => return Err(RuntimeV4ExpertRestActionParseError::InvalidValue),
    }
    if selection && option == "mend" && !identity(&witness["target_player_id"]) {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}

fn validate_card_evidence(
    evidence: &Value,
    option: &str,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let evidence = evidence
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    exact_fields(
        evidence,
        &["kind", "added_card_ids", "removed_card_ids", "upgraded_card_ids"],
    )?;
    if evidence["kind"] != "card_change" {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let added = bounded_unique_ids(&evidence["added_card_ids"])?;
    let removed = bounded_unique_ids(&evidence["removed_card_ids"])?;
    let upgraded = bounded_unique_ids(&evidence["upgraded_card_ids"])?;
    let changed = match option {
        "clone" => added.is_empty(),
        "cook" => removed.is_empty(),
        "smith" => upgraded.is_empty(),
        _ => true,
    };
    if changed {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}

fn validate_relic_evidence(
    evidence: &Value,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let evidence = evidence
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    exact_fields(evidence, &["kind", "added_relic_ids", "removed_relic_ids"])?;
    if evidence["kind"] != "relic_change" {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    if bounded_unique_ids(&evidence["added_relic_ids"])?.is_empty() {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    bounded_unique_ids(&evidence["removed_relic_ids"])?;
    Ok(())
}

fn validate_hp_evidence(
    evidence: &Value,
    require_increase: bool,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let evidence = evidence
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    exact_fields(
        evidence,
        &["kind", "hp_before", "hp_after", "max_hp_before", "max_hp_after"],
    )?;
    if evidence["kind"] != "hp_change" {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let before = bounded_u16(&evidence["hp_before"])?;
    let after = bounded_u16(&evidence["hp_after"])?;
    let max_before = bounded_u16(&evidence["max_hp_before"])?;
    let max_after = bounded_u16(&evidence["max_hp_after"])?;
    if before > max_before || after > max_after || (require_increase && after <= before) {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}

fn validate_stat_evidence(
    evidence: &Value,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let evidence = evidence
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    exact_fields(evidence, &["kind", "stat_id", "before", "after"])?;
    if evidence["kind"] != "stat_change"
        || !identity(&evidence["stat_id"])
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let before = signed_bounded(&evidence["before"])?;
    let after = signed_bounded(&evidence["after"])?;
    if before == after {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}

fn validate_native_evidence(
    evidence: &Value,
    state_id: &str,
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    let evidence = evidence
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    exact_fields(evidence, &["kind", "completion_id", "native_state_id"])?;
    if evidence["kind"] != "native_completion"
        || !identity(&evidence["completion_id"])
        || !identity(&evidence["native_state_id"])
        || evidence["native_state_id"] != state_id
    {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}
