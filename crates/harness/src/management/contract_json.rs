// SPDX-License-Identifier: MIT

//! Versioned management wire contracts and strict boundary decoding.
//!
//! The management adapter deliberately owns only transport and control-plane
//! shapes.  Definition semantics, scheduling, and effect settlement remain
//! behind the ports in `service.rs`.

use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MANAGEMENT_SCHEMA_VERSION: &str = "ascension.management/v1";
pub const RUN_SCHEMA_VERSION: &str = "ascension.workflow-run/v1";
pub const EVENT_SCHEMA_VERSION: &str = "ascension.workflow-event/v1";
pub const STATUS_SCHEMA_VERSION: &str = "ascension.workflow-status/v1";
pub const REPLAY_SCHEMA_VERSION: &str = "ascension.workflow-replay/v1";
pub const EXPORT_SCHEMA_VERSION: &str = "ascension.workflow-export/v1";
pub const CAPABILITIES_SCHEMA_VERSION: &str = "ascension.capabilities/v1";

pub const MAX_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
pub const MAX_PATH_BYTES: usize = 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MAX_IDENTIFIER_BYTES: usize = 128;
pub const MAX_STRING_BYTES: usize = 4 * 1024;
pub const MAX_JSON_DEPTH: usize = 32;
pub const MAX_JSON_ITEMS: usize = 16 * 1024;
pub const MAX_EVENTS_PER_PAGE: u64 = 128;
pub const MAX_EVENTS_PER_RUN: usize = 8192;
pub const MAX_STORE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CONNECTIONS: usize = 16;
pub const REQUEST_DEADLINE_MILLIS: u64 = 5_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError {
    pub code: String,
    pub message: String,
}

impl ContractError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for ContractError {}

/// Decode JSON while retaining duplicate-key rejection before a typed shape is
/// deserialized. `serde_json::Value` by itself keeps the last duplicate key.
pub fn decode_strict<T>(bytes: &[u8]) -> Result<T, ContractError>
where
    T: DeserializeOwned,
{
    if bytes.len() > MAX_JSON_BYTES {
        return Err(ContractError::new(
            "body_too_large",
            "JSON body exceeds the management limit",
        ));
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let StrictValue(value) = StrictValue::deserialize(&mut deserializer)
        .map_err(|error| ContractError::new("invalid_json", error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| ContractError::new("invalid_json", error.to_string()))?;
    validate_value_limits(&value, 0, &mut 0)?;
    serde_json::from_value(value)
        .map_err(|error| ContractError::new("invalid_shape", error.to_string()))
}

/// Decode an arbitrary JSON document with the same strict parser used for
/// typed requests. This is used by injected definition/compiler ports.
pub fn decode_value(bytes: &[u8]) -> Result<Value, ContractError> {
    decode_strict(bytes)
}

fn validate_value_limits(
    value: &Value,
    depth: usize,
    item_count: &mut usize,
) -> Result<(), ContractError> {
    if depth > MAX_JSON_DEPTH {
        return Err(ContractError::new(
            "json_too_deep",
            "JSON nesting exceeds the management limit",
        ));
    }
    *item_count = item_count
        .checked_add(1)
        .ok_or_else(|| ContractError::new("json_too_large", "JSON item count overflowed"))?;
    if *item_count > MAX_JSON_ITEMS {
        return Err(ContractError::new(
            "json_too_large",
            "JSON item count exceeds the management limit",
        ));
    }
    match value {
        Value::String(text) => {
            if text.len() > MAX_STRING_BYTES {
                return Err(ContractError::new(
                    "string_too_large",
                    "JSON string exceeds the management limit",
                ));
            }
        }
        Value::Number(number) => {
            if let Some(integer) = number.as_u64()
                && integer > 9_007_199_254_740_991
            {
                return Err(ContractError::new(
                    "unsafe_integer",
                    "JSON integer exceeds the exact management range",
                ));
            }
            if let Some(integer) = number.as_i64()
                && integer.unsigned_abs() > 9_007_199_254_740_991
            {
                return Err(ContractError::new(
                    "unsafe_integer",
                    "JSON integer exceeds the exact management range",
                ));
            }
            if let Some(float) = number.as_f64()
                && !float.is_finite()
            {
                return Err(ContractError::new(
                    "non_finite_number",
                    "non-finite numbers are not valid JSON",
                ));
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_value_limits(child, depth + 1, item_count)?;
            }
        }
        Value::Object(values) => {
            for (key, child) in values {
                if key.len() > MAX_STRING_BYTES {
                    return Err(ContractError::new(
                        "string_too_large",
                        "JSON object key exceeds the management limit",
                    ));
                }
                validate_value_limits(child, depth + 1, item_count)?;
            }
        }
        Value::Null | Value::Bool(_) => {}
    }
    Ok(())
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

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a strict JSON value")
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
                Ok(StrictValue(Value::Number(Number::from(value))))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictValue(Value::Number(Number::from(value))))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Number::from_f64(value)
                    .map(|number| StrictValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("non-finite number"))
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

            fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = access.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate object key"));
                    }
                    let value = access.next_value::<StrictValue>()?;
                    values.insert(key, value.0);
                }
                Ok(StrictValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(StrictVisitor)
    }
}

pub fn digest_value(value: &Value) -> Result<String, ContractError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ContractError::new("canonicalization_failed", error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn validate_identifier(field: &str, value: &str) -> Result<(), ContractError> {
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES {
        return Err(ContractError::new(
            "invalid_identifier",
            format!("{field} must be 1..={MAX_IDENTIFIER_BYTES} bytes"),
        ));
    }
    let mut chars = value.chars();
    let first = chars
        .next()
        .ok_or_else(|| ContractError::new("invalid_identifier", format!("{field} is empty")))?;
    if !first.is_ascii_alphanumeric()
        || !chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
    {
        return Err(ContractError::new(
            "invalid_identifier",
            format!("{field} contains an unsupported character"),
        ));
    }
    Ok(())
}

pub fn validate_digest(field: &str, value: &str) -> Result<(), ContractError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ContractError::new(
            "invalid_digest",
            format!("{field} must be a lowercase SHA-256 digest"),
        ));
    }
    Ok(())
}

pub fn schema_is(value: &str, expected: &str) -> Result<(), ContractError> {
    if value != expected {
        return Err(ContractError::new(
            "unsupported_schema",
            format!("expected schema {expected}"),
        ));
    }
    Ok(())
}
