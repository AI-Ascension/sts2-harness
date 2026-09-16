// SPDX-License-Identifier: MIT

fn decode_lookup_binding_session_response(
    context: &LookupBindingContext,
    bytes: &[u8],
    expected_correlation: &str,
    retained: Option<&LookupObservation>,
) -> Result<Decoded, LookupBindingError> {
    let value = decode_lookup_binding_response(bytes)?;
    if value["protocol_version"] != LOOKUP_BINDING_PROFILE
        || value["schema_digest"] != LOOKUP_BINDING_SCHEMA_DIGEST
    {
        return Err(LookupBindingError::UnsupportedVersion);
    }
    schema(&value)?;
    if value["correlation_id"] != expected_correlation {
        return Err(LookupBindingError::Invalid);
    }
    let binding = &value["binding"];
    if binding.is_null() {
        return Err(error_code(&value));
    }
    let supplied = binding["binding_id"]
        .as_str()
        .ok_or(LookupBindingError::Invalid)?;
    let expected = binding_id(binding)?;
    if supplied != expected {
        return Err(LookupBindingError::InvalidIdentity);
    }
    if binding["scope"] != json!(context.scope)
        || binding["instance_id"] != context.instance_id
        || binding["authority_epoch"] != context.authority_epoch
    {
        return Err(LookupBindingError::DeniedScope);
    }
    let required_profile = value["discovery"]["required_capabilities"]["profile"]
        .as_str()
        .ok_or(LookupBindingError::Invalid)?;
    if value["discovery"]["required_capabilities"]["schema_digest"]
        != LOOKUP_BINDING_SCHEMA_DIGEST
        || !context
            .supported_capabilities
            .iter()
            .any(|capability| capability == required_profile)
    {
        return Err(LookupBindingError::MissingCapability);
    }
    let decoded = LookupBinding {
        binding_id: supplied.to_owned(),
        game_profile: string(binding, "game_profile")?,
        content_manifest_id: string(binding, "content_manifest_id")?,
        locale: string(binding, "locale")?,
    };
    let observation = if value["observation"].is_null() {
        None
    } else {
        let observation = &value["observation"];
        if observation["binding_id"] != supplied {
            return Err(LookupBindingError::MixedBinding);
        }
        Some(LookupObservation {
            observation_id: string(observation, "observation_id")?,
            snapshot_id: string(observation, "snapshot_id")?,
            state_generation: observation["state_generation"]
                .as_u64()
                .ok_or(LookupBindingError::Invalid)?,
        })
    };
    let state = string(&value["discovery"], "observation_state")?;
    let supersedes = value["discovery"]["reobserve"]["supersedes_observation_id"].as_str();
    if let Some(retained) = retained
        && matches!(state.as_str(), "reobserve_required" | "reobserve_exhausted")
        && supersedes != Some(retained.observation_id.as_str())
    {
        return Err(LookupBindingError::Invalid);
    }
    if let (Some(retained), Some(observation)) = (retained, observation.as_ref())
        && observation.state_generation < retained.state_generation
    {
        return Err(LookupBindingError::StaleSnapshot);
    }
    let kind = string(&value, "kind")?;
    let error = if kind == "error_response" {
        let code = error_code(&value);
        if code != LookupBindingError::ReobserveUnavailable
            || state != "reobserve_exhausted"
            || observation.is_some()
            || value["discovery"]["reobserve"]["attempts"]
                .as_u64()
                .is_none_or(|attempts| attempts < 1)
        {
            return Err(LookupBindingError::Invalid);
        }
        Some(code)
    } else {
        None
    };
    Ok(Decoded {
        kind,
        state,
        binding: decoded,
        observation,
        error,
    })
}
