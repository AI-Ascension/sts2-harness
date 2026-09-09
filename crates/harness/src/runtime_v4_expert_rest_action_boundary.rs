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
