// SPDX-License-Identifier: MIT

fn parse_strict(bytes: &[u8]) -> Result<Value, RuntimeV4ExpertActionParseError> {
    if bytes.len() > MAX_ACTION_BYTES {
        return Err(RuntimeV4ExpertActionParseError::TooLarge);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let StrictValue(value) = StrictValue::deserialize(&mut deserializer)
        .map_err(|_| RuntimeV4ExpertActionParseError::MalformedJson)?;
    deserializer
        .end()
        .map_err(|_| RuntimeV4ExpertActionParseError::MalformedJson)?;
    Ok(value)
}
fn validate_root(
    value: &Value,
    kind: &str,
    _request: Option<&RuntimeV4ExpertActionRequest>,
) -> Result<(), RuntimeV4ExpertActionParseError> {
    let Some(root) = value.as_object() else {
        return Err(RuntimeV4ExpertActionParseError::InvalidShape);
    };
    if root.len() != ROOT_FIELDS.len() || ROOT_FIELDS.iter().any(|field| !root.contains_key(*field))
    {
        return Err(RuntimeV4ExpertActionParseError::InvalidShape);
    }
    if value["protocol_version"] != RUNTIME_V4_EXPERT_ACTION_PROTOCOL_VERSION
        || value["schema_digest"] != RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST
        || value["profile"] != "expert-action"
        || value["kind"] != kind
    {
        return Err(RuntimeV4ExpertActionParseError::InvalidValue);
    }
    let Some(provenance) = value["provenance"].as_object() else {
        return Err(RuntimeV4ExpertActionParseError::InvalidShape);
    };
    if provenance.len() != 3
        || provenance["artifact"] != RUNTIME_V4_EXPERT_ACTION_ARTIFACT
        || provenance["source"] != RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE
        || provenance["generator"] != RUNTIME_V4_EXPERT_ACTION_GENERATOR
    {
        return Err(RuntimeV4ExpertActionParseError::InvalidValue);
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
            return Err(RuntimeV4ExpertActionParseError::InvalidValue);
        }
    }
    for field in ["lease_epoch", "generation"] {
        if value[field]
            .as_u64()
            .is_none_or(|number| number > MAX_SAFE_INTEGER)
        {
            return Err(RuntimeV4ExpertActionParseError::InvalidValue);
        }
    }
    Ok(())
}

fn validate_request(value: &Value) -> Result<(), RuntimeV4ExpertActionParseError> {
    if !value["status"].is_null()
        || !value["observation"].is_null()
        || !value["transition"].is_null()
        || !value["error_code"].is_null()
    {
        return Err(RuntimeV4ExpertActionParseError::InvalidValue);
    }
    validate_action_reference(&value["action"])
}

fn validate_response(
    value: &Value,
) -> Result<RuntimeV4ExpertActionStatus, RuntimeV4ExpertActionParseError> {
    let status = value["status"]
        .as_str()
        .and_then(RuntimeV4ExpertActionStatus::parse)
        .ok_or(RuntimeV4ExpertActionParseError::InvalidValue)?;
    match status {
        RuntimeV4ExpertActionStatus::Accepted => {
            validate_action_reference(&value["action"])?;
            require_null(&value["observation"])?;
            require_null(&value["transition"])?;
            require_null(&value["error_code"])?;
        }
        RuntimeV4ExpertActionStatus::Settled => {
            validate_action_reference(&value["action"])?;
            let observation = value["observation"].clone();
            let observation = crate::RuntimeV4ExpertObservation::from_value(observation)
                .map_err(|_| RuntimeV4ExpertActionParseError::InvalidValue)?;
            if observation.state_id() != value["state_id"]
                || observation.generation() != value["generation"].as_u64().unwrap_or(0)
            {
                return Err(RuntimeV4ExpertActionParseError::InvalidValue);
            }
            let Some(transition) = value["transition"].as_object() else {
                return Err(RuntimeV4ExpertActionParseError::InvalidShape);
            };
            let fields = [
                "kind",
                "before_generation",
                "after_generation",
                "potion_id",
                "removed",
            ];
            if transition.len() != fields.len()
                || fields.iter().any(|field| !transition.contains_key(*field))
            {
                return Err(RuntimeV4ExpertActionParseError::InvalidShape);
            }
            if transition["kind"] != "potion_use_settled"
                || transition["removed"] != true
                || transition["before_generation"]
                    .as_u64()
                    .is_none_or(|number| number > MAX_SAFE_INTEGER)
                || transition["after_generation"]
                    .as_u64()
                    .is_none_or(|number| number > MAX_SAFE_INTEGER)
                || transition["after_generation"].as_u64()
                    <= transition["before_generation"].as_u64()
                || transition["after_generation"] != value["generation"]
                || transition["potion_id"] != value["action"]["action"]["potion_id"]
            {
                return Err(RuntimeV4ExpertActionParseError::InvalidValue);
            }
            require_null(&value["error_code"])?;
        }
        RuntimeV4ExpertActionStatus::Rejected
        | RuntimeV4ExpertActionStatus::Unknown
        | RuntimeV4ExpertActionStatus::Cancelled => {
            if !value["action"].is_null() {
                validate_action_reference(&value["action"])?;
            }
            require_null(&value["observation"])?;
            require_null(&value["transition"])?;
            if !identity(&value["error_code"]) {
                return Err(RuntimeV4ExpertActionParseError::InvalidValue);
            }
        }
    }
    Ok(status)
}

fn validate_action_reference(value: &Value) -> Result<(), RuntimeV4ExpertActionParseError> {
    let Some(action) = value.as_object() else {
        return Err(RuntimeV4ExpertActionParseError::InvalidShape);
    };
    if action.len() != 2 || !action.contains_key("action_id") || !action.contains_key("action") {
        return Err(RuntimeV4ExpertActionParseError::InvalidShape);
    }
    if !identity(&action["action_id"]) {
        return Err(RuntimeV4ExpertActionParseError::InvalidValue);
    }
    let Some(payload) = action["action"].as_object() else {
        return Err(RuntimeV4ExpertActionParseError::InvalidShape);
    };
    if payload.len() != 3
        || payload["kind"] != "use_potion"
        || !identity(&payload["potion_id"])
        || !(payload["target_id"].is_null() || identity(&payload["target_id"]))
    {
        return Err(RuntimeV4ExpertActionParseError::InvalidValue);
    }
    Ok(())
}

fn require_null(value: &Value) -> Result<(), RuntimeV4ExpertActionParseError> {
    value
        .is_null()
        .then_some(())
        .ok_or(RuntimeV4ExpertActionParseError::InvalidValue)
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
