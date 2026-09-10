// SPDX-License-Identifier: MIT

use super::artifact::verify_coop_native_candidate_artifact;
use serde::{Deserialize, Deserializer, de::{self, MapAccess, SeqAccess, Visitor}};
use serde_json::Map;

impl CoopNativeEnvelope {
    /// Parses a response-sized envelope and rejects a request/response direction mismatch.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CoopNativeEnvelopeError> {
        parse_with_limit(bytes, COOP_NATIVE_MAX_RESPONSE_BYTES, None)
    }

    /// Alias for callers that use the parser terminology.
    pub fn parse(bytes: &[u8]) -> Result<Self, CoopNativeEnvelopeError> {
        Self::from_bytes(bytes)
    }

    pub fn from_json(text: &str) -> Result<Self, CoopNativeEnvelopeError> {
        Self::from_bytes(text.as_bytes())
    }

    pub fn parse_request(bytes: &[u8]) -> Result<Self, CoopNativeEnvelopeError> {
        parse_with_limit(bytes, COOP_NATIVE_MAX_REQUEST_BYTES, Some(true))
    }

    pub fn parse_response(bytes: &[u8]) -> Result<Self, CoopNativeEnvelopeError> {
        parse_with_limit(bytes, COOP_NATIVE_MAX_RESPONSE_BYTES, Some(false))
    }

    pub fn from_value(value: Value) -> Result<Self, CoopNativeEnvelopeError> {
        let bytes = serde_json::to_vec(&value).map_err(|_| CoopNativeEnvelopeError::MalformedJson)?;
        Self::from_bytes(&bytes)
    }

    /// Serializes the validated semantic value without exposing any unparsed input.
    pub fn to_json(&self) -> Result<String, CoopNativeEnvelopeError> {
        serde_json::to_string(&self.value).map_err(|_| CoopNativeEnvelopeError::MalformedJson)
    }
}

fn parse_with_limit(
    bytes: &[u8],
    limit: usize,
    expected_request: Option<bool>,
) -> Result<CoopNativeEnvelope, CoopNativeEnvelopeError> {
    if bytes.len() > limit {
        return Err(CoopNativeEnvelopeError::TooLarge);
    }
    verify_coop_native_candidate_artifact()
        .map_err(|_| CoopNativeEnvelopeError::ArtifactMismatch)?;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let StrictValue(value) = StrictValue::deserialize(&mut deserializer).map_err(|error| {
        if error.to_string().contains("duplicate JSON object member") {
            CoopNativeEnvelopeError::DuplicateMember
        } else {
            CoopNativeEnvelopeError::MalformedJson
        }
    })?;
    deserializer
        .end()
        .map_err(|_| CoopNativeEnvelopeError::MalformedJson)?;
    if !within_depth(&value, 0) {
        return Err(CoopNativeEnvelopeError::DepthExceeded);
    }
    let envelope = parse_envelope(value)?;
    let actual_request = match envelope.kind() {
        CoopNativeKind::RecoveryResponse => envelope
            .recovery_response()
            .and_then(CoopNativeRecoveryResponse::status)
            .is_none(),
        kind => kind.is_request(),
    };
    if expected_request.is_some_and(|request| request != actual_request) {
        return Err(CoopNativeEnvelopeError::WrongDirection);
    }
    Ok(envelope)
}

fn within_depth(value: &Value, depth: usize) -> bool {
    if depth > COOP_NATIVE_MAX_DEPTH {
        return false;
    }
    match value {
        Value::Array(values) => values.iter().all(|value| within_depth(value, depth + 1)),
        Value::Object(values) => values.values().all(|value| within_depth(value, depth + 1)),
        _ => true,
    }
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
                formatter.write_str("JSON with unique object members")
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
                        return Err(de::Error::custom("duplicate JSON object member"));
                    }
                    object.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(object)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

include!("coop_native_wire_rules.rs");
