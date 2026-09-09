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
        || positive_count(&value["required_count"])? > MAX_SELECTOR_ITEMS
        || bounded_unique_ids(&value["selected_choice_ids"])?.len() > MAX_SELECTOR_ITEMS
        || bounded_count(&value["remaining_count"])?
            != positive_count(&value["required_count"])?
                .saturating_sub(bounded_unique_ids(&value["selected_choice_ids"])?.len())
    {
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
    let evidence = witness["evidence"]
        .as_object()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    let evidence_kind = evidence["kind"]
        .as_str()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
    let expected_evidence = match option {
        "heal" | "mend" => ["hp_change", "native_completion"].as_slice(),
        "smith" | "clone" | "cook" => ["card_change"].as_slice(),
        "dig" | "hatch" => ["relic_change"].as_slice(),
        "lift" => ["stat_change", "native_completion"].as_slice(),
        "kindle" => ["native_completion"].as_slice(),
        _ => [].as_slice(),
    };
    if !expected_evidence.contains(&evidence_kind) {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    if selection && option == "mend" && !identity(&witness["target_player_id"]) {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    Ok(())
}

fn exact_fields(
    value: &Map<String, Value>,
    fields: &[&str],
) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    if value.len() != fields.len() || fields.iter().any(|field| !value.contains_key(*field)) {
        Err(RuntimeV4ExpertRestActionParseError::InvalidShape)
    } else {
        Ok(())
    }
}
fn require_null(value: &Value) -> Result<(), RuntimeV4ExpertRestActionParseError> {
    value
        .is_null()
        .then_some(())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}
fn number(value: &Value) -> Result<u64, RuntimeV4ExpertRestActionParseError> {
    value
        .as_u64()
        .filter(|number| *number <= MAX_SAFE_INTEGER)
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}
fn positive_count(value: &Value) -> Result<usize, RuntimeV4ExpertRestActionParseError> {
    number(value)
        .ok()
        .filter(|value| (1..=MAX_SELECTOR_ITEMS as u64).contains(value))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}
fn bounded_count(value: &Value) -> Result<usize, RuntimeV4ExpertRestActionParseError> {
    number(value)
        .ok()
        .filter(|value| *value <= MAX_SELECTOR_ITEMS as u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)
}
fn bounded_unique_ids(value: &Value) -> Result<Vec<&str>, RuntimeV4ExpertRestActionParseError> {
    let ids = value
        .as_array()
        .ok_or(RuntimeV4ExpertRestActionParseError::InvalidShape)?;
    if ids.len() > MAX_SELECTOR_ITEMS {
        return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id
            .as_str()
            .ok_or(RuntimeV4ExpertRestActionParseError::InvalidValue)?;
        if !identity(&Value::String(id.to_owned())) || !seen.insert(id) {
            return Err(RuntimeV4ExpertRestActionParseError::InvalidValue);
        }
        result.push(id);
    }
    Ok(result)
}
fn identity(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        !value.is_empty()
            && value.len() <= MAX_IDENTITY_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
    })
}
