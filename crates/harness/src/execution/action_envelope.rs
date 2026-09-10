// SPDX-License-Identifier: MIT

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fmt;

/// Validates the generic durable action envelope without depending on a runtime-v3 binary.
/// Runtime-specific action semantics remain at the runtime adapter boundary.
pub(super) fn validate_canonical_action_envelope(
    expected_action_id: &str,
    bytes: &[u8],
    max_bytes: usize,
) -> bool {
    if bytes.is_empty() || bytes.len() > max_bytes {
        return false;
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueValue
        .deserialize(&mut deserializer)
        .ok()
        .filter(|_| deserializer.end().is_ok());
    let Some(value) = value else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 2
        || object.get("action_id").and_then(Value::as_str) != Some(expected_action_id)
        || !object.get("action").is_some_and(Value::is_object)
    {
        return false;
    }
    serde_json::to_vec(&value).is_ok_and(|canonical| canonical == bytes)
}

struct UniqueValue;

impl<'de> DeserializeSeed<'de> for UniqueValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for UniqueValue {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object fields")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(UniqueValue)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        let mut names = HashSet::new();
        while let Some(name) = access.next_key::<String>()? {
            if !names.insert(name.clone()) {
                return Err(de::Error::custom("duplicate JSON object field"));
            }
            let value = access.next_value_seed(UniqueValue)?;
            object.insert(name, value);
        }
        Ok(Value::Object(object))
    }
}
