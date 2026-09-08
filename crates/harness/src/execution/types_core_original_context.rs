// SPDX-License-Identifier: MIT

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde_json::Value;
use std::fmt;

pub const MAX_ORIGINAL_CONTEXT_BYTES: usize = 8 * 1024;

/// representation bounded and structurally strict before it can be used for a sideband request.
pub(crate) fn valid_original_context_raw(raw: &[u8]) -> bool {
    if raw.is_empty() || raw.len() > MAX_ORIGINAL_CONTEXT_BYTES {
        return false;
    }
    let Ok(object) = unique_context_object(raw) else {
        return false;
    };
    let expected = [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "lease_id",
        "lease_epoch",
    ];
    object.len() == expected.len()
        && expected.iter().all(|key| object.contains_key(*key))
        && object
            .get("deployment_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid)
        && object
            .get("instance_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid)
        && object
            .get("instance_incarnation")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid_v4)
        && object
            .get("boot_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid_v4)
        && object
            .get("lease_id")
            .and_then(Value::as_str)
            .is_some_and(valid_context_uuid_v4)
        && object
            .get("authority_generation")
            .and_then(Value::as_u64)
            .is_some_and(valid_context_integer)
        && object
            .get("lease_epoch")
            .and_then(Value::as_u64)
            .is_some_and(valid_context_integer)
}

fn unique_context_object(raw: &[u8]) -> Result<serde_json::Map<String, Value>, ()> {
    struct ContextVisitor;

    impl<'de> Visitor<'de> for ContextVisitor {
        type Value = serde_json::Map<String, Value>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a closed JSON object without duplicate members")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut object = serde_json::Map::new();
            while let Some(key) = access.next_key::<String>()? {
                if object.contains_key(&key) {
                    return Err(de::Error::custom("duplicate original context member"));
                }
                object.insert(key, access.next_value()?);
            }
            Ok(object)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(raw);
    let object = deserializer
        .deserialize_map(ContextVisitor)
        .map_err(|_| ())?;
    deserializer.end().map_err(|_| ())?;
    Ok(object)
}

fn valid_context_uuid(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23)
                    && (byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        })
        && matches!(value.as_bytes().get(19), Some(b'8' | b'9' | b'a' | b'b'))
}

fn valid_context_uuid_v4(value: &str) -> bool {
    valid_context_uuid(value) && value.as_bytes().get(14) == Some(&b'4')
}

fn valid_context_integer(value: u64) -> bool {
    (1..=9_007_199_254_740_991).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::{MAX_ORIGINAL_CONTEXT_BYTES, valid_original_context_raw};

    const VALID: &[u8] = br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1}"#;

    #[test]
    fn original_context_is_closed_bounded_and_duplicate_free() {
        assert!(valid_original_context_raw(VALID));
        assert!(!valid_original_context_raw(
            br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1}"#
        ));
        assert!(!valid_original_context_raw(
            br#"{"deployment_id":"33333333-3333-4333-8333-333333333333","instance_id":"44444444-4444-4444-8444-444444444444","instance_incarnation":"55555555-5555-4555-8555-555555555555","boot_id":"66666666-6666-4666-8666-666666666666","authority_generation":1,"lease_id":"77777777-7777-4777-8777-777777777777","lease_epoch":1,"extra":true}"#
        ));
        let mut oversized = VALID.to_vec();
        oversized.resize(MAX_ORIGINAL_CONTEXT_BYTES + 1, b' ');
        assert!(!valid_original_context_raw(&oversized));
    }
}
