// SPDX-License-Identifier: MIT

//! Closed-field parsing helpers for the Exo capability contract.

use super::ExoPreflightError;

/// Parses one bounded JSON object, rejecting duplicate keys.
pub(super) fn parse_unique_object(
    bytes: &[u8],
) -> Result<serde_json::Map<String, serde_json::Value>, ExoPreflightError> {
    struct UniqueFields;

    impl<'de> serde::de::Visitor<'de> for UniqueFields {
        type Value = serde_json::Map<String, serde_json::Value>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a capability object with unique field names")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut object = serde_json::Map::new();
            while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                if object.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate capability field"));
                }
            }
            Ok(object)
        }
    }

    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let object = serde::Deserializer::deserialize_map(&mut decoder, UniqueFields)
        .map_err(|_| ExoPreflightError::Malformed)?;
    decoder.end().map_err(|_| ExoPreflightError::Malformed)?;
    Ok(object)
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
