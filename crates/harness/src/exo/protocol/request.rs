// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use super::ExoError;
use crate::exo::sandbox::SanitizedObservation;
use crate::identity::ModelExecutionId;

const MAX_ACTION_IDS: usize = 256;
const MAX_CONSTRAINTS: usize = 32;
const MAX_CONSTRAINT_BYTES: usize = 512;

/// A structured fair-play decision request sent to Exo.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ExoDecisionRequest {
    pub schema: String,
    pub provider_revision: String,
    pub model_execution_id: String,
    pub state_id: String,
    pub generation: u64,
    pub observation: serde_json::Value,
    pub legal_action_ids: Vec<String>,
    pub objective: String,
    pub hard_constraints: Vec<String>,
    pub max_response_bytes: u32,
}

impl ExoDecisionRequest {
    /// Builds a request after validating projection, IDs, and bounded prompt constraints.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution_id: ModelExecutionId,
        provider_revision: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        observation: SanitizedObservation,
        legal_action_ids: Vec<String>,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
        max_response_bytes: usize,
    ) -> Result<Self, ExoError> {
        let state_id = state_id.into();
        let provider_revision = provider_revision.into();
        let objective = objective.into();
        if !valid_revision(&provider_revision)
            || !valid_id(&state_id)
            || generation > 9_007_199_254_740_991
            || !valid_action_ids(&legal_action_ids)
            || !legal_action_ids_match(observation.as_value(), &legal_action_ids)
            || !valid_text(&objective)
            || hard_constraints.len() > MAX_CONSTRAINTS
            || hard_constraints.iter().any(|value| !valid_text(value))
            || max_response_bytes == 0
            || max_response_bytes > 8 * 1024
            || max_response_bytes > u32::MAX as usize
        {
            return Err(ExoError::InvalidRequest);
        }
        Ok(Self {
            schema: "sts2.exo-decision-v1".to_owned(),
            provider_revision,
            model_execution_id: execution_id.to_string(),
            state_id,
            generation,
            observation: observation.as_value().clone(),
            legal_action_ids,
            objective,
            hard_constraints,
            max_response_bytes: max_response_bytes as u32,
        })
    }

    pub fn encode(&self, max_request_bytes: usize) -> Result<Vec<u8>, ExoError> {
        validate_request(self)?;
        let bytes = serde_json::to_vec(self).map_err(|_| ExoError::InvalidRequest)?;
        if bytes.len() > max_request_bytes {
            return Err(ExoError::RequestTooLarge);
        }
        Ok(bytes)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

pub(super) fn valid_revision(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

fn valid_action_ids(values: &[String]) -> bool {
    if values.is_empty() || values.len() > MAX_ACTION_IDS {
        return false;
    }
    let mut unique = BTreeSet::new();
    values
        .iter()
        .all(|value| valid_id(value) && unique.insert(value.as_str()))
}

fn legal_action_ids_match(observation: &serde_json::Value, requested: &[String]) -> bool {
    let Some(actions) = observation
        .get("legal_actions")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    actions.len() == requested.len()
        && actions.iter().zip(requested).all(|(action, requested_id)| {
            action.get("action_id").and_then(serde_json::Value::as_str)
                == Some(requested_id.as_str())
        })
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_CONSTRAINT_BYTES && !value.chars().any(char::is_control)
}

fn validate_request(request: &ExoDecisionRequest) -> Result<(), ExoError> {
    if request.schema != "sts2.exo-decision-v1"
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
    Ok(())
}

pub(super) fn request_from_prompt(
    execution_id: ModelExecutionId,
    provider_revision: &str,
    prompt: &str,
    max_response_bytes: usize,
) -> Result<ExoDecisionRequest, ExoError> {
    let value: serde_json::Value =
        serde_json::from_str(prompt).map_err(|_| ExoError::MalformedResponse)?;
    let object = value.as_object().ok_or(ExoError::InvalidRequest)?;
    const ALLOWED: [&str; 6] = [
        "observation",
        "state_id",
        "generation",
        "legal_action_ids",
        "objective",
        "hard_constraints",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ExoError::InvalidRequest);
    }
    let observation = object
        .get("observation")
        .cloned()
        .ok_or(ExoError::InvalidRequest)
        .and_then(|value| SanitizedObservation::new(value).map_err(ExoError::Sandbox))?;
    let state_id = object
        .get("state_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoError::InvalidRequest)?;
    let generation = object
        .get("generation")
        .and_then(serde_json::Value::as_u64)
        .ok_or(ExoError::InvalidRequest)?;
    let legal_action_ids = request_text_list(object, "legal_action_ids")?;
    let objective = object
        .get("objective")
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoError::InvalidRequest)?;
    let hard_constraints = request_text_list(object, "hard_constraints")?;
    ExoDecisionRequest::new(
        execution_id,
        provider_revision,
        state_id,
        generation,
        observation,
        legal_action_ids,
        objective,
        hard_constraints,
        max_response_bytes,
    )
}

fn request_text_list(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, ExoError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_array)
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
