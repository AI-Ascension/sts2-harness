// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) const MAX_CONTROL_ID_BYTES: usize = 128;
const MAX_CONTROL_FIELDS: usize = 8;

/// Closed outer request envelope. Correlation lives here, not in the model-facing body.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoBridgeRequestEnvelope {
    pub wire_version: String,
    pub request_id: String,
    pub turn_id: String,
    pub request: super::super::ExoDecisionRequest,
}

/// Closed terminal response envelope; the inner decision remains unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoBridgeDecisionEnvelope {
    pub wire_version: String,
    pub request_id: String,
    pub turn_id: String,
    pub outcome: ExoWireOutcome,
    pub decision: Option<Value>,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoWireOutcome {
    Decision,
    Cancelled,
    Failed,
}

/// Process/session identity held by the harness control plane, never placed in model prompts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoControlIdentity {
    pub run_id: String,
    pub episode_id: String,
    pub model_execution_id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub idempotency_key: String,
}

impl ExoControlIdentity {
    pub fn validate(&self) -> Result<(), super::wire::ExoWireError> {
        let values = [
            self.run_id.as_str(),
            self.episode_id.as_str(),
            self.model_execution_id.as_str(),
            self.agent_id.as_str(),
            self.conversation_id.as_str(),
            self.session_id.as_str(),
            self.turn_id.as_str(),
            self.idempotency_key.as_str(),
        ];
        if values.len() > MAX_CONTROL_FIELDS || values.iter().any(|value| !valid_control_id(value))
        {
            return Err(super::wire::ExoWireError::InvalidIdentity);
        }
        Ok(())
    }
}

/// Host-owned terminal lifecycle states; these are control records, not model responses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExoTerminalOutcome {
    Decision,
    Cancelled,
    Failed,
}

/// Correlated turn receipt retained by the harness after one terminal exchange.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoBridgeTurn {
    pub identity: ExoControlIdentity,
    pub outcome: ExoTerminalOutcome,
}

impl ExoBridgeTurn {
    pub fn validate(&self) -> Result<(), super::wire::ExoWireError> {
        self.identity.validate()
    }
}

/// Strict bridge framing errors. No error variant carries model output or credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExoWireError {
    TooLarge,
    InvalidUtf8,
    MalformedJson,
    TrailingBytes,
    DuplicateField,
    UnknownField,
    InvalidShape,
    InvalidRequest,
    VersionMismatch,
    Cancelled,
    RemoteFailure,
    Decision(super::super::DecisionError),
    InvalidIdentity,
    IdentityMismatch,
}

impl std::fmt::Display for ExoWireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "Exo bridge frame exceeds its byte bound",
            Self::InvalidUtf8 => "Exo bridge frame is not UTF-8",
            Self::MalformedJson => "Exo bridge frame is malformed JSON",
            Self::TrailingBytes => "Exo bridge frame contains trailing bytes",
            Self::DuplicateField => "Exo bridge frame contains a duplicate field",
            Self::UnknownField => "Exo bridge frame contains an unknown field",
            Self::InvalidShape => "Exo bridge frame has an invalid JSON shape",
            Self::InvalidRequest => "Exo bridge request failed the STS2 request contract",
            Self::VersionMismatch => "Exo bridge envelope version is unsupported",
            Self::Cancelled => "Exo bridge turn was cancelled",
            Self::RemoteFailure => "Exo bridge turn failed",
            Self::Decision(_) => "Exo bridge decision failed strict terminal parsing",
            Self::InvalidIdentity => "Exo bridge control identity is invalid",
            Self::IdentityMismatch => "Exo bridge control identities do not match",
        })
    }
}

impl std::error::Error for ExoWireError {}

impl From<super::super::DecisionError> for ExoWireError {
    fn from(error: super::super::DecisionError) -> Self {
        Self::Decision(error)
    }
}

fn valid_control_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONTROL_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
