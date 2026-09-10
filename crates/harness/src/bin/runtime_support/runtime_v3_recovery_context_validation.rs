// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

fn required_recovery_env(config: &RuntimeConfig, name: &str) -> Result<String, String> {
    // Values are captured in RuntimeConfig before the process environment is scrubbed for child
    // MCP processes. Falling back to the parent environment preserves the live CLI path while
    // allowing tests and embedded callers to provide an isolated configuration directly.
    config
        .recovery_value(name)
        .map(str::to_owned)
        .or_else(|| std::env::var(name).ok())
        .ok_or_else(|| format!("{name} is required for the recovery sideband"))
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
    valid_uuid(value)
        && value.as_bytes().get(14) == Some(&b'4')
        && value
            .as_bytes()
            .get(19)
            .is_some_and(|byte| matches!(*byte, b'8' | b'9' | b'a' | b'b'))
}

fn validate_fence(
    fence: &Value,
    deployment_id: &str,
    instance_id: &str,
    instance_incarnation: &str,
    boot_id: &str,
    authority_generation: u64,
) -> Result<(), String> {
    let object = fence
        .as_object()
        .ok_or_else(|| String::from("recovery current fence is not an object"))?;
    let expected: BTreeSet<&str> = [
        "host_fence_id",
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "fence_generation",
        "created_at",
    ]
    .into_iter()
    .collect();
    if object.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
        || object.get("deployment_id").and_then(Value::as_str) != Some(deployment_id)
        || object.get("instance_id").and_then(Value::as_str) != Some(instance_id)
        || object.get("instance_incarnation").and_then(Value::as_str) != Some(instance_incarnation)
        || object.get("boot_id").and_then(Value::as_str) != Some(boot_id)
        || object.get("authority_generation").and_then(Value::as_u64) != Some(authority_generation)
        || !positive_wire_integer(authority_generation)
        || !object
            .get("host_fence_id")
            .and_then(Value::as_str)
            .is_some_and(valid_uuid_v4)
        || !object
            .get("fence_generation")
            .and_then(Value::as_u64)
            .is_some_and(positive_wire_integer)
        || !object
            .get("created_at")
            .and_then(Value::as_str)
            .is_some_and(strict_timestamp)
    {
        return Err(String::from("recovery current fence is invalid"));
    }
    Ok(())
}

fn positive_wire_integer(value: u64) -> bool {
    (1..=MAX_WIRE_INTEGER).contains(&value)
}

fn strict_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(20..=30).contains(&bytes.len())
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || bytes.last() != Some(&b'Z')
    {
        return false;
    }
    if !bytes[..19]
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit())
    {
        return false;
    }
    if bytes.len() == 20 {
        return timestamp_parts(bytes, None);
    }
    if bytes.get(19) != Some(&b'.')
        || !bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit)
        || !(1..=9).contains(&(bytes.len() - 21))
    {
        return false;
    }
    timestamp_parts(bytes, Some(bytes.len() - 21))
}

fn timestamp_parts(bytes: &[u8], fraction_digits: Option<usize>) -> bool {
    if bytes.last() != Some(&b'Z')
        || fraction_digits.is_some_and(|digits| !(1..=9).contains(&digits))
    {
        return false;
    }
    let year = timestamp_number(&bytes[0..4]);
    let month = timestamp_number(&bytes[5..7]);
    let day = timestamp_number(&bytes[8..10]);
    let hour = timestamp_number(&bytes[11..13]);
    let minute = timestamp_number(&bytes[14..16]);
    let second = timestamp_number(&bytes[17..19]);
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) =
        (year, month, day, hour, minute, second)
    else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=12).contains(&month)
        && (1..=days).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60
}

fn timestamp_number(bytes: &[u8]) -> Option<u32> {
    bytes.iter().all(u8::is_ascii_digit).then(|| {
        bytes
            .iter()
            .fold(0_u32, |value, byte| value * 10 + u32::from(byte - b'0'))
    })
}

#[cfg(test)]
mod tests {
    use super::{MAX_WIRE_INTEGER, positive_wire_integer, strict_timestamp};

    #[test]
    fn fence_numbers_and_timestamps_are_strict_and_bounded() {
        assert!(positive_wire_integer(1));
        assert!(!positive_wire_integer(0));
        assert!(!positive_wire_integer(MAX_WIRE_INTEGER + 1));
        assert!(strict_timestamp("2026-02-28T23:59:59.123456789Z"));
        assert!(!strict_timestamp("2026-02-29T23:59:59Z"));
        assert!(!strict_timestamp("2026-02-28T23:59:59+00:00"));
    }
}
