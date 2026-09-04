// SPDX-License-Identifier: MIT

use crate::error::PortError;
use crate::identity::{ActionId, ModelExecutionId, RecordId};
use crate::records::Correlation;
use serde_json::Value;

const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_ID_BYTES: usize = 512;

/// Evidence labels are deliberately explicit; unavailable evidence is not a successful result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceStatus {
    Confirmed,
    SourceDerived,
    Inferred,
    Proposed,
    Unverified,
    Unsupported,
}

impl EvidenceStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::SourceDerived => "source-derived",
            Self::Inferred => "inferred",
            Self::Proposed => "proposed",
            Self::Unverified => "unverified",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Semantic classes retained by the bounded decision memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionRecordKind {
    Observation,
    Request,
    Acceptance,
    Settlement,
    Recovery,
    Estimate,
    Unavailable,
}

impl DecisionRecordKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Request => "request",
            Self::Acceptance => "acceptance",
            Self::Settlement => "settlement",
            Self::Recovery => "recovery",
            Self::Estimate => "estimate",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Structured, sanitized payload bytes. Verbatim provider responses are not an accepted input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionPayload(Vec<u8>);

impl DecisionPayload {
    pub fn from_json(value: Value) -> Result<Self, PortError> {
        if contains_forbidden_field(&value) {
            return Err(PortError::new(
                "privileged_decision_payload",
                "decision payload contains a forbidden field",
                false,
            ));
        }
        let bytes = serde_json::to_vec(&value).map_err(|_| {
            PortError::new(
                "decision_payload_encoding",
                "decision payload could not be encoded",
                false,
            )
        })?;
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(PortError::new(
                "decision_payload_too_large",
                "decision payload exceeds its bound",
                false,
            ));
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One typed trajectory fact. It contains no raw model output or host object reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRecord {
    record_id: RecordId,
    sequence: u64,
    correlation: Correlation,
    kind: DecisionRecordKind,
    evidence: EvidenceStatus,
    generation: Option<u64>,
    state_id: Option<String>,
    operation_id: Option<String>,
    action_id: Option<ActionId>,
    model_execution_id: Option<ModelExecutionId>,
    payload: DecisionPayload,
}

impl DecisionRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_id: RecordId,
        sequence: u64,
        correlation: Correlation,
        kind: DecisionRecordKind,
        evidence: EvidenceStatus,
        generation: Option<u64>,
        state_id: Option<String>,
        operation_id: Option<String>,
        action_id: Option<ActionId>,
        model_execution_id: Option<ModelExecutionId>,
        payload: DecisionPayload,
    ) -> Result<Self, PortError> {
        for value in [&state_id, &operation_id]
            .into_iter()
            .flatten()
        {
            if !valid_identity(value) {
                return Err(PortError::new(
                    "invalid_decision_identity",
                    "decision record identity is invalid",
                    false,
                ));
            }
        }
        if generation.is_some_and(|value| value > 9_007_199_254_740_991) {
            return Err(PortError::new(
                "invalid_decision_generation",
                "decision record generation exceeds its bound",
                false,
            ));
        }
        if correlation.model_execution_id() != model_execution_id {
            return Err(PortError::new(
                "decision_model_identity_mismatch",
                "decision record model identity does not match correlation",
                false,
            ));
        }
        Ok(Self {
            record_id,
            sequence,
            correlation,
            kind,
            evidence,
            generation,
            state_id,
            operation_id,
            action_id,
            model_execution_id,
            payload,
        })
    }

    #[must_use]
    pub const fn record_id(&self) -> RecordId { self.record_id }

    #[must_use]
    pub const fn sequence(&self) -> u64 { self.sequence }

    #[must_use]
    pub const fn correlation(&self) -> &Correlation { &self.correlation }

    #[must_use]
    pub const fn kind(&self) -> DecisionRecordKind { self.kind }

    #[must_use]
    pub const fn evidence(&self) -> EvidenceStatus { self.evidence }

    #[must_use]
    pub const fn generation(&self) -> Option<u64> { self.generation }

    #[must_use]
    pub fn state_id(&self) -> Option<&str> { self.state_id.as_deref() }

    #[must_use]
    pub fn operation_id(&self) -> Option<&str> { self.operation_id.as_deref() }

    #[must_use]
    pub const fn action_id(&self) -> Option<ActionId> { self.action_id }

    #[must_use]
    pub const fn model_execution_id(&self) -> Option<ModelExecutionId> {
        self.model_execution_id
    }

    #[must_use]
    pub const fn payload(&self) -> &DecisionPayload { &self.payload }
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn contains_forbidden_field(value: &Value) -> bool {
    const FORBIDDEN: [&str; 17] = [
        "raw_memory", "host_object", "random_state", "future_rng", "unrevealed",
        "credential", "password", "access_token", "private_prompt", "executable",
        "process_command", "reflection", "screen_coordinate", "input_event", "save_file",
        "pck", "dll",
    ];
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            FORBIDDEN.iter().any(|name| key.eq_ignore_ascii_case(name))
                || contains_forbidden_field(child)
        }),
        Value::Array(values) => values.iter().any(contains_forbidden_field),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}
