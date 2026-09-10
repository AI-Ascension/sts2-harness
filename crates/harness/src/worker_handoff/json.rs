// SPDX-License-Identifier: MIT

use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

use super::{HandoffError, MAX_FRAME_BYTES, MAX_INTEGER};

pub(super) fn decode(bytes: &[u8]) -> Result<Value, HandoffError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(HandoffError);
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Bounded(0)
        .deserialize(&mut decoder)
        .map_err(|_| HandoffError)?;
    decoder.end().map_err(|_| HandoffError)?;
    Ok(value)
}

struct Bounded(usize);

impl<'de> DeserializeSeed<'de> for Bounded {
    type Value = Value;

    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        if self.0 > 16 {
            return Err(D::Error::custom("depth limit"));
        }
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Bounded {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded worker JSON")
    }

    fn visit_bool<E: Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_str<E: Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_u64<E: Error>(self, value: u64) -> Result<Value, E> {
        if value > MAX_INTEGER {
            return Err(E::custom("integer limit"));
        }
        Ok(Value::from(value))
    }

    // Default signed and floating-point visitors reject negative, fractional,
    // exponent and negative-zero spellings before semantic validation.
    fn visit_seq<A: SeqAccess<'de>>(self, mut array: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = array.next_element_seed(Bounded(self.0 + 1))? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom("duplicate field"));
            }
            let value = object.next_value_seed(Bounded(self.0 + 1))?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn depth_is_checked_before_command_field_validation() {
        for depth in [16, 17] {
            let bytes = format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(super::decode(bytes.as_bytes()).is_ok(), depth == 16);
        }
    }

    #[test]
    fn nested_duplicates_and_noncanonical_numbers_do_not_reach_semantic_use() {
        for bytes in [
            br#"{"nested":{"a":1,"\u0061":2}}"#.as_slice(),
            br#"[1e0]"#.as_slice(),
            br#"{"nested":-0}"#.as_slice(),
        ] {
            assert!(super::decode(bytes).is_err());
        }
    }
}
