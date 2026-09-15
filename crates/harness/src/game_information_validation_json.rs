// SPDX-License-Identifier: MIT
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::fmt;

struct Unique(Value);

impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Value;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bounded JSON with unique members")
            }
            fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
                Ok(Value::Bool(value))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
                Ok(value.into())
            }
            fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
                Ok(value.into())
            }
            fn visit_f64<E: de::Error>(self, _value: f64) -> Result<Value, E> {
                Err(E::custom("integer encoding required"))
            }
            fn visit_str<E>(self, value: &str) -> Result<Value, E> {
                Ok(value.into())
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = input.next_element::<Unique>()? {
                    values.push(value.0);
                }
                Ok(Value::Array(values))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Value, A::Error> {
                let mut object = Map::new();
                while let Some(key) = input.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(de::Error::custom("duplicate member"));
                    }
                    object.insert(key, input.next_value::<Unique>()?.0);
                }
                Ok(Value::Object(object))
            }
        }
        decoder.deserialize_any(JsonVisitor).map(Unique)
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Unique::deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value.0)
}
