// SPDX-License-Identifier: MIT

//! Closed-field parsing helpers for the Exo capability contract.
//!
//! Duplicate object keys are rejected at every nesting level, not only at the descriptor root.

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

use super::ExoPreflightError;

struct UniqueValue(serde_json::Value);

impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value whose objects have unique keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        let number = serde_json::Number::from_f64(value);
        Ok(UniqueValue(number.map_or(
            serde_json::Value::Null,
            serde_json::Value::Number,
        )))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        UniqueValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueValue>()? {
            values.push(value.0);
        }
        Ok(UniqueValue(serde_json::Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = serde_json::Map::new();
        while let Some((key, value)) = map.next_entry::<String, UniqueValue>()? {
            if object.insert(key, value.0).is_some() {
                return Err(serde::de::Error::custom("duplicate capability field"));
            }
        }
        Ok(UniqueValue(serde_json::Value::Object(object)))
    }
}

/// Parses one bounded JSON object, rejecting duplicate keys at every level.
pub(super) fn parse_unique_object(
    bytes: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, ExoPreflightError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value =
        UniqueValue::deserialize(&mut deserializer).map_err(|_| ExoPreflightError::Malformed)?;
    deserializer
        .end()
        .map_err(|_| ExoPreflightError::Malformed)?;
    match value.0 {
        serde_json::Value::Object(object) => Ok(object),
        _ => Err(ExoPreflightError::Malformed),
    }
}

pub(super) fn string_field<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, ExoPreflightError> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or(ExoPreflightError::UnsupportedValue)
}

pub(super) fn string_list(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, ExoPreflightError> {
    let values = object
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or(ExoPreflightError::UnsupportedValue)?;
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let text = value.as_str().ok_or(ExoPreflightError::UnsupportedValue)?;
        out.push(text.to_owned());
    }
    Ok(out)
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn valid_revision(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}
