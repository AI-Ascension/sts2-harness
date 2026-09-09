// SPDX-License-Identifier: MIT

use super::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES};
use super::{ExoConfig, ExoError};
use crate::episode::map::MapDecisionContext;
use crate::exo::sandbox::SanitizedObservation;
use crate::identity::ModelExecutionId;

#[path = "request_validation.rs"]
mod validation;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_context: Option<serde_json::Value>,
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
        Self::new_inner(
            execution_id,
            provider_revision,
            state_id,
            generation,
            observation,
            legal_action_ids,
            objective,
            hard_constraints,
            max_response_bytes,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_map(
        execution_id: ModelExecutionId,
        provider_revision: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        observation: SanitizedObservation,
        legal_action_ids: Vec<String>,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
        max_response_bytes: usize,
        map_context: MapDecisionContext,
    ) -> Result<Self, ExoError> {
        Self::new_inner(
            execution_id,
            provider_revision,
            state_id,
            generation,
            observation,
            legal_action_ids,
            objective,
            hard_constraints,
            max_response_bytes,
            Some(map_context.to_wire()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        execution_id: ModelExecutionId,
        provider_revision: impl Into<String>,
        state_id: impl Into<String>,
        generation: u64,
        observation: SanitizedObservation,
        legal_action_ids: Vec<String>,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
        max_response_bytes: usize,
        map_context: Option<serde_json::Value>,
    ) -> Result<Self, ExoError> {
        let state_id = state_id.into();
        let provider_revision = provider_revision.into();
        let objective = objective.into();
        let schema = if map_context.is_some() {
            "sts2.exo-decision-map-v1"
        } else {
            "sts2.exo-decision-v1"
        };
        if !valid_revision(&provider_revision)
            || !validation::valid_id(&state_id)
            || generation > 9_007_199_254_740_991
            || !validation::valid_action_ids(&legal_action_ids)
            || !validation::legal_action_ids_match(observation.as_value(), &legal_action_ids)
            || !validation::valid_text(&objective)
            || hard_constraints.len() > MAX_CONSTRAINTS
            || hard_constraints
                .iter()
                .any(|value| !validation::valid_text(value))
            || max_response_bytes == 0
            || max_response_bytes > 8 * 1024
            || max_response_bytes > u32::MAX as usize
        {
            return Err(ExoError::InvalidRequest);
        }
        let request = Self {
            schema: schema.to_owned(),
            provider_revision,
            model_execution_id: execution_id.to_string(),
            state_id,
            generation,
            observation: observation.as_value().clone(),
            legal_action_ids,
            objective,
            hard_constraints,
            max_response_bytes: max_response_bytes as u32,
            map_context,
        };
        validation::validate_request(&request)?;
        Ok(request)
    }

    pub fn encode(&self, max_request_bytes: usize) -> Result<Vec<u8>, ExoError> {
        validation::validate_request(self)?;
        let bytes = serde_json::to_vec(self).map_err(|_| ExoError::InvalidRequest)?;
        let effective_limit = if self.map_context.is_some() {
            max_request_bytes.min(EXO_MAX_MAP_REQUEST_BYTES)
        } else {
            max_request_bytes.min(EXO_MAX_STANDARD_REQUEST_BYTES)
        };
        if bytes.len() > effective_limit {
            return Err(ExoError::RequestTooLarge);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "../../../tests/support/exo_request_real_mcp_tests.rs"]
mod real_mcp_tests;

pub(super) fn valid_revision(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

pub(super) fn request_from_prompt(
    execution_id: ModelExecutionId,
    config: &ExoConfig,
    prompt: &str,
) -> Result<ExoDecisionRequest, ExoError> {
    let value: serde_json::Value =
        serde_json::from_str(prompt).map_err(|_| ExoError::MalformedResponse)?;
    let object = value.as_object().ok_or(ExoError::InvalidRequest)?;
    const ALLOWED: [&str; 7] = [
        "observation",
        "state_id",
        "generation",
        "legal_action_ids",
        "objective",
        "hard_constraints",
        "map_context",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ExoError::InvalidRequest);
    }
    let observation = object
        .get("observation")
        .cloned()
        .ok_or(ExoError::InvalidRequest)
        .and_then(|value| SanitizedObservation::new(value).map_err(ExoError::Sandbox))
        .map(|observation| config.project(observation))?;
    let state_id = object
        .get("state_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoError::InvalidRequest)?;
    let generation = object
        .get("generation")
        .and_then(serde_json::Value::as_u64)
        .ok_or(ExoError::InvalidRequest)?;
    let legal_action_ids = validation::request_text_list(object, "legal_action_ids")?;
    let objective = object
        .get("objective")
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoError::InvalidRequest)?;
    let hard_constraints = validation::request_text_list(object, "hard_constraints")?;
    ExoDecisionRequest::from_prompt_parts(
        execution_id,
        config,
        state_id,
        generation,
        observation,
        legal_action_ids,
        objective,
        hard_constraints,
        object.get("map_context").cloned(),
    )
}

impl ExoDecisionRequest {
    #[allow(clippy::too_many_arguments)]
    fn from_prompt_parts(
        execution_id: ModelExecutionId,
        config: &ExoConfig,
        state_id: &str,
        generation: u64,
        observation: SanitizedObservation,
        legal_action_ids: Vec<String>,
        objective: &str,
        hard_constraints: Vec<String>,
        map_context: Option<serde_json::Value>,
    ) -> Result<Self, ExoError> {
        Self::new_inner(
            execution_id,
            config.revision.as_str(),
            state_id,
            generation,
            observation,
            legal_action_ids,
            objective,
            hard_constraints,
            config.max_response_bytes,
            map_context,
        )
    }
}
