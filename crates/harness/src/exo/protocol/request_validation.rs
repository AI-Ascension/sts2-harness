// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::Value;

use super::{
    ExoDecisionRequest, ExoError, MAX_ACTION_IDS, MAX_CONSTRAINT_BYTES, MAX_CONSTRAINTS,
    valid_revision,
};
use crate::episode::map::MapDecisionContext;
use crate::exo::sandbox::SanitizedObservation;

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

pub(super) fn valid_action_ids(values: &[String]) -> bool {
    if values.is_empty() || values.len() > MAX_ACTION_IDS {
        return false;
    }
    let mut unique = BTreeSet::new();
    values
        .iter()
        .all(|value| valid_id(value) && unique.insert(value.as_str()))
}

pub(super) fn legal_action_ids_match(observation: &Value, requested: &[String]) -> bool {
    let Some(actions) = observation.get("legal_actions").and_then(Value::as_array) else {
        return false;
    };
    actions.len() == requested.len()
        && actions.iter().zip(requested).all(|(action, requested_id)| {
            action.get("action_id").and_then(Value::as_str) == Some(requested_id.as_str())
        })
}

pub(super) fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_CONSTRAINT_BYTES && !value.chars().any(char::is_control)
}

pub(super) fn validate_request(request: &ExoDecisionRequest) -> Result<(), ExoError> {
    if (request.schema != "sts2.exo-decision-v1" && request.schema != "sts2.exo-decision-map-v1")
        || (request.schema == "sts2.exo-decision-v1" && request.map_context.is_some())
        || (request.schema == "sts2.exo-decision-map-v1" && request.map_context.is_none())
        || !valid_revision(&request.provider_revision)
        || !valid_id(&request.model_execution_id)
        || !valid_id(&request.state_id)
        || request.generation > 9_007_199_254_740_991
        || !valid_action_ids(&request.legal_action_ids)
        || !legal_action_ids_match(&request.observation, &request.legal_action_ids)
        || !valid_text(&request.objective)
        || request.hard_constraints.len() > MAX_CONSTRAINTS
        || request
            .hard_constraints
            .iter()
            .any(|value| !valid_text(value))
        || request.max_response_bytes == 0
        || request.max_response_bytes > 8 * 1024
    {
        return Err(ExoError::InvalidRequest);
    }
    let observation =
        SanitizedObservation::new(request.observation.clone()).map_err(ExoError::Sandbox)?;
    if observation.state_id() != Some(request.state_id.as_str())
        || observation.generation() != Some(request.generation)
    {
        return Err(ExoError::InvalidRequest);
    }
    if let Some(map_context) = &request.map_context {
        MapDecisionContext::from_exo_value(
            map_context,
            &request.state_id,
            request.generation,
            &request.legal_action_ids,
        )
        .map_err(|_| ExoError::InvalidRequest)?;
    }
    Ok(())
}

pub(super) fn request_text_list(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, ExoError> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or(ExoError::InvalidRequest)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(ExoError::InvalidRequest)
        })
        .collect()
}
