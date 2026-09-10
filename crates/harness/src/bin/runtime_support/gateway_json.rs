// SPDX-License-Identifier: MIT

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fmt;

pub(super) const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_JSON_DEPTH: usize = 64;

pub(super) fn parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.is_empty() || bytes.len() > MAX_JSON_BYTES {
        return Err(String::from("gateway response JSON exceeded its bound"));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueValue { depth: 0 }
        .deserialize(&mut deserializer)
        .map_err(|_| String::from("gateway response was not JSON"))?;
    deserializer
        .end()
        .map_err(|_| String::from("gateway response contained trailing JSON"))?;
    Ok(value)
}

struct UniqueValue {
    depth: usize,
}

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
        formatter.write_str("a bounded JSON value without duplicate object fields")
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
        let depth = self.next_depth()?;
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(UniqueValue { depth })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let depth = self.next_depth()?;
        let mut object = Map::new();
        let mut names = HashSet::new();
        while let Some(name) = access.next_key::<String>()? {
            if !names.insert(name.clone()) {
                return Err(de::Error::custom("duplicate JSON object field"));
            }
            let value = access.next_value_seed(UniqueValue { depth })?;
            object.insert(name, value);
        }
        Ok(Value::Object(object))
    }
}

impl UniqueValue {
    fn next_depth<E: de::Error>(&self) -> Result<usize, E> {
        let depth = self
            .depth
            .checked_add(1)
            .filter(|depth| *depth <= MAX_JSON_DEPTH)
            .ok_or_else(|| E::custom("JSON nesting exceeded its bound"))?;
        Ok(depth)
    }
}

#[cfg(test)]
#[path = "gateway_json_tests.rs"]
mod tests;
