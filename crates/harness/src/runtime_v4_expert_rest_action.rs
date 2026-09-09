// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

use crate::runtime_v4_expert_rest_action_artifact::{
    RUNTIME_V4_EXPERT_REST_ACTION_ARTIFACT, RUNTIME_V4_EXPERT_REST_ACTION_GENERATOR,
    RUNTIME_V4_EXPERT_REST_ACTION_PROTOCOL_VERSION, RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_DIGEST,
    RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_SOURCE, verify_runtime_v4_expert_rest_action_artifact,
};

const MAX_ACTION_BYTES: usize = 128 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_IDENTITY_BYTES: usize = 512;
const MAX_SELECTOR_ITEMS: usize = 256;

const ROOT_FIELDS: [&str; 19] = [
    "protocol_version",
    "schema_digest",
    "provenance",
    "profile",
    "correlation_id",
    "instance_id",
    "session_id",
    "lease_id",
    "lease_epoch",
    "generation",
    "state_id",
    "operation_id",
    "kind",
    "action",
    "status",
    "observation",
    "transition",
    "effect_witness",
    "error_code",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertRestActionParseError {
    TooLarge,
    MalformedJson,
    InvalidShape,
    InvalidValue,
    ArtifactMismatch,
    IdentityMismatch,
}

impl std::fmt::Display for RuntimeV4ExpertRestActionParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "runtime-v4 expert REST action exceeds its byte bound",
            Self::MalformedJson => "runtime-v4 expert REST action is malformed JSON",
            Self::InvalidShape => "runtime-v4 expert REST action has an invalid closed shape",
            Self::InvalidValue => "runtime-v4 expert REST action has an invalid value",
            Self::ArtifactMismatch => "runtime-v4 expert REST action artifact verification failed",
            Self::IdentityMismatch => "runtime-v4 expert REST action identities do not match",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertRestActionParseError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertRestActionStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

impl RuntimeV4ExpertRestActionStatus {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "accepted" => Self::Accepted,
            "settled" => Self::Settled,
            "rejected" => Self::Rejected,
            "unknown" => Self::Unknown,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV4ExpertRestActionRequest {
    value: Value,
}

impl RuntimeV4ExpertRestActionRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeV4ExpertRestActionParseError> {
        let value = parse_strict(bytes)?;
        validate_root(&value, "action_request")?;
        validate_request(&value)?;
        verify_runtime_v4_expert_rest_action_artifact()
            .map_err(|_| RuntimeV4ExpertRestActionParseError::ArtifactMismatch)?;
        Ok(Self { value })
    }

    pub fn from_value(value: Value) -> Result<Self, RuntimeV4ExpertRestActionParseError> {
        let encoded = serde_json::to_vec(&value)
            .map_err(|_| RuntimeV4ExpertRestActionParseError::MalformedJson)?;
        Self::parse(&encoded)
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub fn operation_id(&self) -> &str {
        self.value["operation_id"].as_str().unwrap_or("")
    }

    #[must_use]
    pub fn action_id(&self) -> &str {
        self.value["action"]["action_id"].as_str().unwrap_or("")
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.value["generation"].as_u64().unwrap_or(0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV4ExpertRestActionResult {
    value: Value,
    status: RuntimeV4ExpertRestActionStatus,
}

impl RuntimeV4ExpertRestActionResult {
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeV4ExpertRestActionParseError> {
        let value = parse_strict(bytes)?;
        validate_root(&value, "action_response")?;
        let status = validate_response(&value)?;
        verify_runtime_v4_expert_rest_action_artifact()
            .map_err(|_| RuntimeV4ExpertRestActionParseError::ArtifactMismatch)?;
        Ok(Self { value, status })
    }

    pub fn from_value(value: Value) -> Result<Self, RuntimeV4ExpertRestActionParseError> {
        let encoded = serde_json::to_vec(&value)
            .map_err(|_| RuntimeV4ExpertRestActionParseError::MalformedJson)?;
        Self::parse(&encoded)
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub const fn status(&self) -> RuntimeV4ExpertRestActionStatus {
        self.status
    }

    #[must_use]
    pub fn operation_id(&self) -> &str {
        self.value["operation_id"].as_str().unwrap_or("")
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.value["generation"].as_u64().unwrap_or(0)
    }

    #[must_use]
    pub fn transition_kind(&self) -> Option<&str> {
        self.value["transition"].get("kind").and_then(Value::as_str)
    }

    #[must_use]
    pub fn transition(&self) -> Option<&Value> {
        (!self.value["transition"].is_null()).then_some(&self.value["transition"])
    }

    #[must_use]
    pub fn observation(&self) -> Option<&Value> {
        (!self.value["observation"].is_null()).then_some(&self.value["observation"])
    }

    #[must_use]
    pub fn effect_witness(&self) -> Option<&Value> {
        (!self.value["effect_witness"].is_null()).then_some(&self.value["effect_witness"])
    }

    /// Bind a response, including a read-only reconcile response, to its original operation.
    pub fn matches_request(
        &self,
        request: &RuntimeV4ExpertRestActionRequest,
    ) -> Result<(), RuntimeV4ExpertRestActionParseError> {
        for field in [
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
            "operation_id",
        ] {
            if self.value[field] != request.value[field] {
                return Err(RuntimeV4ExpertRestActionParseError::IdentityMismatch);
            }
        }
        if self.value["action"] != request.value["action"] {
            return Err(RuntimeV4ExpertRestActionParseError::IdentityMismatch);
        }
        if self.status == RuntimeV4ExpertRestActionStatus::Settled
            && self.value["transition"]["before_generation"] != request.value["generation"]
        {
            return Err(RuntimeV4ExpertRestActionParseError::IdentityMismatch);
        }
        if self.status != RuntimeV4ExpertRestActionStatus::Settled
            && (self.value["state_id"] != request.value["state_id"]
                || self.value["generation"] != request.value["generation"])
        {
            return Err(RuntimeV4ExpertRestActionParseError::IdentityMismatch);
        }
        Ok(())
    }
}

include!("runtime_v4_expert_rest_action_boundary.rs");
include!("runtime_v4_expert_rest_action_reference.rs");
include!("runtime_v4_expert_rest_action_witness.rs");
include!("runtime_v4_expert_rest_action_settled.rs");
include!("runtime_v4_expert_rest_action_validation_helpers.rs");
include!("runtime_v4_expert_rest_action_strict.rs");

#[cfg(test)]
mod tests {
    include!("runtime_v4_expert_rest_action_tests.rs");
}
