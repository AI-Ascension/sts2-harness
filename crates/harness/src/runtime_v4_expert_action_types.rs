// SPDX-License-Identifier: MIT

const ROOT_FIELDS: [&str; 18] = [
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
    "error_code",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertActionParseError {
    TooLarge,
    MalformedJson,
    InvalidShape,
    InvalidValue,
    ArtifactMismatch,
    IdentityMismatch,
}
impl std::fmt::Display for RuntimeV4ExpertActionParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "runtime-v4-expert action exceeds its byte bound",
            Self::MalformedJson => "runtime-v4-expert action is malformed JSON",
            Self::InvalidShape => "runtime-v4-expert action has an invalid closed shape",
            Self::InvalidValue => "runtime-v4-expert action has an invalid value",
            Self::ArtifactMismatch => "runtime-v4-expert action artifact verification failed",
            Self::IdentityMismatch => "runtime-v4-expert action identities do not match",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertActionParseError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertActionStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

impl RuntimeV4ExpertActionStatus {
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
pub struct RuntimeV4ExpertActionRequest {
    value: Value,
}

impl RuntimeV4ExpertActionRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeV4ExpertActionParseError> {
        let value = parse_strict(bytes)?;
        validate_root(&value, "action_request", None)?;
        validate_request(&value)?;
        verify_runtime_v4_expert_action_artifact()
            .map_err(|_| RuntimeV4ExpertActionParseError::ArtifactMismatch)?;
        Ok(Self { value })
    }

    pub fn from_value(value: Value) -> Result<Self, RuntimeV4ExpertActionParseError> {
        let encoded = serde_json::to_vec(&value)
            .map_err(|_| RuntimeV4ExpertActionParseError::MalformedJson)?;
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
pub struct RuntimeV4ExpertActionResult {
    value: Value,
    status: RuntimeV4ExpertActionStatus,
}

impl RuntimeV4ExpertActionResult {
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeV4ExpertActionParseError> {
        let value = parse_strict(bytes)?;
        validate_root(&value, "action_response", None)?;
        let status = validate_response(&value)?;
        verify_runtime_v4_expert_action_artifact()
            .map_err(|_| RuntimeV4ExpertActionParseError::ArtifactMismatch)?;
        Ok(Self { value, status })
    }

    pub fn from_value(value: Value) -> Result<Self, RuntimeV4ExpertActionParseError> {
        let encoded = serde_json::to_vec(&value)
            .map_err(|_| RuntimeV4ExpertActionParseError::MalformedJson)?;
        Self::parse(&encoded)
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub const fn status(&self) -> RuntimeV4ExpertActionStatus {
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
    pub fn is_settled(&self) -> bool {
        self.status == RuntimeV4ExpertActionStatus::Settled
    }

    /// Binds a response, including a reconciliation response, to the original operation.
    pub fn matches_request(
        &self,
        request: &RuntimeV4ExpertActionRequest,
    ) -> Result<(), RuntimeV4ExpertActionParseError> {
        for field in [
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
            "operation_id",
        ] {
            if self.value[field] != request.value[field] {
                return Err(RuntimeV4ExpertActionParseError::IdentityMismatch);
            }
        }
        if self.status == RuntimeV4ExpertActionStatus::Settled
            && self.value["action"] != request.value["action"]
        {
            return Err(RuntimeV4ExpertActionParseError::IdentityMismatch);
        }
        if self.status == RuntimeV4ExpertActionStatus::Settled
            && self.value["transition"]["before_generation"] != request.value["generation"]
        {
            return Err(RuntimeV4ExpertActionParseError::IdentityMismatch);
        }
        Ok(())
    }
}
