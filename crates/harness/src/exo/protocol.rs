// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use crate::exo::decision::{Decision, DecisionError, parse_decision};
use crate::exo::sandbox::{SanitizedObservation, SandboxError};
use crate::identity::ModelExecutionId;
use crate::provider::{ModelRequest, ModelResponse, ProviderPort};

const MAX_ACTION_IDS: usize = 256;
const MAX_CONSTRAINTS: usize = 32;
const MAX_CONSTRAINT_BYTES: usize = 512;

/// External Exo transport failure; no gameplay fallback is attached to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoTransportError {
    Unavailable,
    Timeout,
    OversizedResponse,
    MalformedResponse,
}

/// Transport owned by the harness adapter. Exo implementation details stay outside this repo.
pub trait ExoTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    )
        -> Result<Vec<u8>, ExoTransportError>;

    fn close(&mut self) -> Result<(), ExoTransportError>;
}

/// Reviewed and bounded adapter configuration. `revision` is mandatory and never inferred.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoConfig {
    pub revision: String,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub timeout_millis: u32,
}

impl ExoConfig {
    pub fn new(
        revision: impl Into<String>,
        max_request_bytes: usize,
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Self, ExoError> {
        let config = Self {
            revision: revision.into(),
            max_request_bytes,
            max_response_bytes,
            timeout_millis,
        };
        if config.revision.is_empty()
            || config.revision.len() > 128
            || !config
                .revision
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
            || config.max_request_bytes == 0
            || config.max_request_bytes > 128 * 1024
            || config.max_response_bytes == 0
            || config.max_response_bytes > 8 * 1024
            || config.timeout_millis == 0
            || config.timeout_millis > 120_000
        {
            return Err(ExoError::InvalidConfig);
        }
        Ok(config)
    }
}

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
        if !valid_id(&provider_revision)
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

/// Adapter-level error retains unavailable/malformed distinctions for fail-closed callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoError {
    InvalidConfig,
    InvalidRequest,
    RequestTooLarge,
    Unavailable,
    Timeout,
    OversizedResponse,
    MalformedResponse,
    Decision(DecisionError),
    Sandbox(SandboxError),
    Closed,
}

impl std::fmt::Display for ExoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfig => "Exo adapter configuration is invalid or unpinned",
            Self::InvalidRequest => "Exo decision request is invalid",
            Self::RequestTooLarge => "Exo decision request exceeds its bound",
            Self::Unavailable => "Exo is unavailable",
            Self::Timeout => "Exo decision timed out",
            Self::OversizedResponse => "Exo response exceeds its bound",
            Self::MalformedResponse => "Exo response is malformed",
            Self::Decision(_) => "Exo decision failed strict validation",
            Self::Sandbox(_) => "observation failed the fair-play firewall",
            Self::Closed => "Exo adapter is closed",
        })
    }
}

impl std::error::Error for ExoError {}

impl From<DecisionError> for ExoError {
    fn from(error: DecisionError) -> Self {
        Self::Decision(error)
    }
}

impl From<SandboxError> for ExoError {
    fn from(error: SandboxError) -> Self {
        Self::Sandbox(error)
    }
}

impl From<ExoTransportError> for ExoError {
    fn from(error: ExoTransportError) -> Self {
        match error {
            ExoTransportError::Unavailable => Self::Unavailable,
            ExoTransportError::Timeout => Self::Timeout,
            ExoTransportError::OversizedResponse => Self::OversizedResponse,
            ExoTransportError::MalformedResponse => Self::MalformedResponse,
        }
    }
}

/// A small transport adapter that keeps Exo behind the existing provider port.
#[derive(Debug)]
pub struct ExoProvider<T> {
    transport: T,
    config: ExoConfig,
    closed: bool,
}

impl<T> ExoProvider<T> {
    pub fn new(transport: T, config: ExoConfig) -> Self {
        Self {
            transport,
            config,
            closed: false,
        }
    }

    #[must_use]
    pub fn config(&self) -> &ExoConfig {
        &self.config
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn execute_request(
        &mut self,
        request: ExoDecisionRequest,
    ) -> Result<Vec<u8>, ExoError>
    where
        T: ExoTransport,
    {
        if self.closed {
            return Err(ExoError::Closed);
        }
        let bytes = request.encode(self.config.max_request_bytes)?;
        self.transport_exchange(&bytes).map_err(ExoError::from)
    }
}

impl<T: ExoTransport> ExoProvider<T> {
    pub(super) fn transport_exchange(
        &mut self,
        request: &[u8],
    ) -> Result<Vec<u8>, ExoTransportError> {
        if self.closed {
            return Err(ExoTransportError::Unavailable);
        }
        let response = self.transport.exchange(
            request,
            self.config.max_response_bytes,
            self.config.timeout_millis,
        )?;
        if response.len() > self.config.max_response_bytes {
            return Err(ExoTransportError::OversizedResponse);
        }
        Ok(response)
    }

    pub(super) fn transport_close(&mut self) -> Result<(), ExoTransportError> {
        if self.closed {
            return Ok(());
        }
        let result = self.transport.close();
        if result.is_ok() {
            self.closed = true;
        }
        result
    }
}

impl<T: ExoTransport> ProviderPort for ExoProvider<T> {
    fn execute(&mut self, request: &ModelRequest) -> Result<ModelResponse, crate::error::ProviderError> {
        let decision_request = request_from_prompt(
            request.execution_id(),
            self.config.revision.as_str(),
            request.prompt().as_str(),
            self.config.max_response_bytes,
        )
        .map_err(|error| provider_error(error_code(error), false))?;
        let output = self
            .execute_request(decision_request)
            .map_err(|error| provider_error(error_code(error), is_retryable(error)))?;
        parse_decision(&output).map_err(|error| provider_error(decision_error_code(error), false))?;
        let output = String::from_utf8(output)
            .map_err(|_| provider_error("exo_malformed_response", false))?;
        let output = crate::provider::ModelOutput::new(output)
            .map_err(|_| provider_error("exo_oversized_response", false))?;
        ModelResponse::new(request.execution_id(), request.correlation().clone(), output)
    }

    fn close(&mut self) -> Result<(), crate::error::PortError> {
        if !self.closed {
            self.transport_close().map_err(|_| {
                crate::error::PortError::new("exo_close_failed", "Exo transport close failed", false)
            })?;
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
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
            action
                .get("action_id")
                .and_then(serde_json::Value::as_str)
                == Some(requested_id.as_str())
        })
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONSTRAINT_BYTES
        && !value.chars().any(char::is_control)
}

fn validate_request(request: &ExoDecisionRequest) -> Result<(), ExoError> {
    if request.schema != "sts2.exo-decision-v1"
        || !valid_id(&request.provider_revision)
        || !valid_id(&request.model_execution_id)
        || !valid_id(&request.state_id)
        || request.generation > 9_007_199_254_740_991
        || !valid_action_ids(&request.legal_action_ids)
        || !legal_action_ids_match(&request.observation, &request.legal_action_ids)
        || !valid_text(&request.objective)
        || request.hard_constraints.len() > MAX_CONSTRAINTS
        || request.hard_constraints.iter().any(|value| !valid_text(value))
        || request.max_response_bytes == 0
        || request.max_response_bytes > 8 * 1024
    {
        return Err(ExoError::InvalidRequest);
    }
    let observation = SanitizedObservation::new(request.observation.clone())
        .map_err(ExoError::Sandbox)?;
    if observation.state_id() != Some(request.state_id.as_str())
        || observation.generation() != Some(request.generation)
    {
        return Err(ExoError::InvalidRequest);
    }
    Ok(())
}

fn request_from_prompt(
    execution_id: ModelExecutionId,
    provider_revision: &str,
    prompt: &str,
    max_response_bytes: usize,
) -> Result<ExoDecisionRequest, ExoError> {
    let value: serde_json::Value = serde_json::from_str(prompt).map_err(|_| ExoError::MalformedResponse)?;
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
    let legal_action_ids = object
        .get("legal_action_ids")
        .and_then(serde_json::Value::as_array)
        .ok_or(ExoError::InvalidRequest)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(ExoError::InvalidRequest)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let objective = object
        .get("objective")
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoError::InvalidRequest)?;
    let hard_constraints = object
        .get("hard_constraints")
        .and_then(serde_json::Value::as_array)
        .ok_or(ExoError::InvalidRequest)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(ExoError::InvalidRequest)
        })
        .collect::<Result<Vec<_>, _>>()?;
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

fn provider_error(code: &'static str, retryable: bool) -> crate::error::ProviderError {
    crate::error::ProviderError::new(code, "Exo adapter rejected or could not complete the request", retryable)
}

fn error_code(error: ExoError) -> &'static str {
    match error {
        ExoError::Unavailable => "exo_unavailable",
        ExoError::Timeout => "exo_timeout",
        ExoError::OversizedResponse => "exo_oversized_response",
        ExoError::MalformedResponse | ExoError::Decision(_) => "exo_malformed_response",
        ExoError::Closed => "exo_closed",
        ExoError::InvalidConfig
        | ExoError::InvalidRequest
        | ExoError::RequestTooLarge
        | ExoError::Sandbox(_) => "exo_invalid_request",
    }
}

fn is_retryable(error: ExoError) -> bool {
    matches!(error, ExoError::Unavailable | ExoError::Timeout)
}

fn decision_error_code(error: DecisionError) -> &'static str {
    match error {
        DecisionError::TooLarge => "exo_oversized_response",
        DecisionError::InvalidJson
        | DecisionError::UnknownField
        | DecisionError::MissingField
        | DecisionError::InvalidValue
        | DecisionError::IllegalAction => "exo_malformed_response",
    }
}
