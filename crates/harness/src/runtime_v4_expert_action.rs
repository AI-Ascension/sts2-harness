// SPDX-License-Identifier: MIT

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

use crate::runtime_v4_expert_action_artifact::{
    RUNTIME_V4_EXPERT_ACTION_ARTIFACT, RUNTIME_V4_EXPERT_ACTION_GENERATOR,
    RUNTIME_V4_EXPERT_ACTION_PROTOCOL_VERSION, RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST,
    RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE, verify_runtime_v4_expert_action_artifact,
};

const MAX_ACTION_BYTES: usize = 128 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_IDENTITY_BYTES: usize = 512;

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
            if observation.generation() != value["generation"].as_u64().unwrap_or(0) {
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

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }
            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::from(value)))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::from(value)))
            }
            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::from(value)))
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::String(value.to_owned())))
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::String(value)))
            }
            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::Null))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::Null))
            }
            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                StrictValue::deserialize(deserializer)
            }
            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut object = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(de::Error::custom("duplicate JSON object key"));
                    }
                    object.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(object)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}
