// SPDX-License-Identifier: MIT

use std::fmt;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

const MAX_AUTH_PROOF_BYTES: usize = 512;

#[path = "runtime_v3_wire_recovery_base64.rs"]
mod base64;
#[path = "runtime_v3_wire_recovery_operation.rs"]
mod operation;
#[path = "runtime_v3_wire_recovery_shape.rs"]
mod shape;
#[path = "runtime_v3_wire_recovery_values.rs"]
mod values;
use values::strict_timestamp;
#[cfg(test)]
#[path = "runtime_v3_wire_recovery_validation_test.rs"]
mod tests;

const FRAME_FIELDS: &[&str] = &[
    "contract",
    "schema_digest",
    "message_id",
    "correlation_id",
    "sent_at",
    "actor",
    "auth",
    "kind",
    "payload",
];

/// Decode a recovery frame from the original bytes. serde_json::Value silently keeps the last
/// value for duplicate object members, so this seed performs the recursive duplicate check before
/// any field-level projection or normalization occurs.
pub(super) fn decode_frame(
    text: &str,
    expected_kind: Option<&str>,
    expected_correlation: Option<&str>,
) -> Result<Value, String> {
    let mut decoder = serde_json::Deserializer::from_slice(text.as_bytes());
    let value = StrictSeed
        .deserialize(&mut decoder)
        .map_err(|error| format!("invalid recovery JSON: {error}"))?;
    decoder
        .end()
        .map_err(|error| format!("trailing recovery JSON: {error}"))?;
    validate_frame(&value, expected_kind, expected_correlation)?;
    Ok(value)
}

struct StrictSeed;

impl<'de> DeserializeSeed<'de> for StrictSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object member names")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictSeed.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON object member {key:?}"
                )));
            }
            let value = map.next_value_seed(StrictSeed)?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

fn validate_frame(
    value: &Value,
    expected_kind: Option<&str>,
    expected_correlation: Option<&str>,
) -> Result<(), String> {
    let object = object(value, "recovery frame")?;
    exact(object, FRAME_FIELDS, "recovery frame")?;
    string_eq(object, "contract", "watchdog-recovery-v1")?;
    string_eq(
        object,
        "schema_digest",
        sts2_harness::RECOVERY_SCHEMA_DIGEST,
    )?;
    let message_id = string(object, "message_id")?;
    if !valid_uuid_v4(message_id) {
        return Err(String::from("recovery message_id is not UUIDv4"));
    }
    let correlation_id = string(object, "correlation_id")?;
    if !valid_uuid_v4(correlation_id) {
        return Err(String::from("recovery correlation_id is not UUIDv4"));
    }
    if let Some(expected) = expected_correlation
        && correlation_id != expected
    {
        return Err(String::from(
            "recovery correlation_id does not echo the request",
        ));
    }
    if !strict_timestamp(string(object, "sent_at")?) {
        return Err(String::from(
            "recovery sent_at is not a strict UTC timestamp",
        ));
    }
    validate_actor(object.get("actor").ok_or("recovery actor is missing")?)?;
    let kind = string(object, "kind")?;
    if let Some(expected) = expected_kind
        && kind != expected
    {
        return Err(format!("recovery kind is not {expected}"));
    }
    validate_auth(object.get("auth").ok_or("recovery auth is missing")?, kind)?;
    validate_payload(
        object.get("payload").ok_or("recovery payload is missing")?,
        kind,
    )
}

fn validate_actor(value: &Value) -> Result<(), String> {
    let object = object(value, "recovery actor")?;
    exact(object, &["principal_id", "role"], "recovery actor")?;
    if !valid_uuid(string(object, "principal_id")?) {
        return Err(String::from("recovery actor principal is invalid"));
    }
    enum_value(
        string(object, "role")?,
        &["gateway", "watchdog", "harness", "host", "mod", "operator"],
        "recovery actor role",
    )
}

fn validate_auth(value: &Value, kind: &str) -> Result<(), String> {
    let object = object(value, "recovery auth")?;
    exact(
        object,
        &["principal_id", "capability", "proof"],
        "recovery auth",
    )?;
    if !valid_uuid(string(object, "principal_id")?) {
        return Err(String::from("recovery auth principal is invalid"));
    }
    let expected = match kind.strip_suffix("_response") {
        Some("operation_lookup") => "recovery_read",
        Some("operation_reconcile") => "recovery_reconcile",
        _ => return Err(String::from("unsupported recovery response kind")),
    };
    string_eq(object, "capability", expected)?;
    match object.get("proof") {
        Some(Value::Null) => Ok(()),
        Some(Value::String(value)) if !value.is_empty() && value.len() <= MAX_AUTH_PROOF_BYTES => {
            Ok(())
        }
        _ => Err(String::from("recovery auth proof is invalid")),
    }
}

fn validate_payload(value: &Value, kind: &str) -> Result<(), String> {
    shape::validate_payload(value, kind)
}

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} is not an object"))
}

fn exact(object: &Map<String, Value>, expected: &[&str], label: &str) -> Result<(), String> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(format!("{label} contains an unknown or missing field"));
    }
    Ok(())
}

fn string<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("recovery field {field} is not a string"))
}

fn string_eq(object: &Map<String, Value>, field: &str, expected: &str) -> Result<(), String> {
    if string(object, field)? != expected {
        return Err(format!("recovery field {field} is not the expected value"));
    }
    Ok(())
}

fn enum_value(value: &str, expected: &[&str], label: &str) -> Result<(), String> {
    if expected.contains(&value) {
        Ok(())
    } else {
        Err(format!("{label} is unsupported"))
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}

fn valid_uuid_v4(value: &str) -> bool {
    valid_uuid(value) && value.as_bytes().get(14) == Some(&b'4')
}
