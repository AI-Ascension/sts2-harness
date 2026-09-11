// SPDX-License-Identifier: MIT

use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SESSION_BINDING_SCHEMA: &str = "ascension.provider-session.binding.v1";
pub const SESSION_CAPABILITIES_SCHEMA: &str = "ascension.provider-session.capabilities.v1";
pub const SESSION_COMPACTION_SCHEMA: &str = "ascension.provider-session.compaction.v1";
pub const SESSION_EVENT_SCHEMA: &str = "ascension.provider-session.event.v1";
pub const SESSION_FORK_SCHEMA: &str = "ascension.provider-session.fork.v1";
pub const SESSION_HISTORY_SCHEMA: &str = "ascension.provider-session.history.v1";
pub const SESSION_OPERATION_SCHEMA: &str = "ascension.provider-session.operation.v1";
pub const SESSION_POLICY_SCHEMA: &str = "ascension.provider-session.policy.v1";
pub const SESSION_PREPARED_SCHEMA: &str = "ascension.provider-session.prepared.v1";
pub const SESSION_RECONCILIATION_SCHEMA: &str = "ascension.provider-session.reconciliation.v1";
pub const SESSION_RETIREMENT_SCHEMA: &str = "ascension.provider-session.retirement.v1";
pub const SESSION_USAGE_SCHEMA: &str = "ascension.provider-session.usage.v1";
/// Version-pinned wire identity for the JSON-RPC 2.0 App Server envelope.
pub const NATIVE_FRAME_SCHEMA: &str = "codex-app-server-jsonrpc.v2";

pub const MAX_SESSION_ITEMS: usize = 512;
pub const MAX_DEPENDENCIES: usize = 128;
pub const MAX_EVENTS: usize = 4096;
pub const MAX_OPERATIONS: usize = 1024;
pub const MAX_PREPARED: usize = 1024;
pub const MAX_CANDIDATES: usize = 4;
pub const MAX_MAINTENANCE_JOBS: usize = 2;
pub const MAX_COMPLETED_TURNS: usize = 128;
pub const MAX_HISTORY_TTL_SECONDS: u64 = 86_400;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PREPARED_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SUFFIX_BYTES: usize = 128 * 1024;
pub const MAX_OUTPUT_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_METHOD_BYTES: usize = 128;
pub const MAX_JSON_DEPTH: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    InvalidScope,
    InvalidPolicy,
    InvalidBinding,
    InvalidCapabilities,
    InvalidOperation,
    InvalidPrepared,
    InvalidRequest,
    Unauthorized,
    AuthNeeded,
    Forbidden,
    NotFound,
    Conflict,
    Stale,
    HeldRequired,
    Retired,
    Capacity,
    Expired,
    Unsupported,
    Ambiguous,
    Fenced,
    Protocol,
    Transport,
    Closed,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidScope => "provider session scope is invalid",
            Self::InvalidPolicy => "provider session policy is invalid",
            Self::InvalidBinding => "provider session binding is invalid",
            Self::InvalidCapabilities => "provider capability profile is invalid",
            Self::InvalidOperation => "provider operation is invalid",
            Self::InvalidPrepared => "prepared provider turn is invalid",
            Self::InvalidRequest => "provider session request is invalid",
            Self::Unauthorized => "provider session authorization is required",
            Self::AuthNeeded => "provider session authentication is needed",
            Self::Forbidden => "provider session operation is forbidden",
            Self::NotFound => "provider session resource is unavailable",
            Self::Conflict => "provider session operation conflicts",
            Self::Stale => "provider session resource is stale",
            Self::HeldRequired => "provider session must remain held",
            Self::Retired => "provider session is retired",
            Self::Capacity => "provider session capacity exceeded",
            Self::Expired => "provider session record has expired",
            Self::Unsupported => "provider session capability is unsupported",
            Self::Ambiguous => "provider operation outcome is ambiguous",
            Self::Fenced => "provider session is fenced",
            Self::Protocol => "provider protocol frame is invalid",
            Self::Transport => "provider transport failed",
            Self::Closed => "provider transport is closed",
        })
    }
}

impl std::error::Error for SessionError {}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

pub(crate) fn valid_method(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_METHOD_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

pub(crate) fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(20..=64).contains(&bytes.len())
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }
    if !bytes[..4]
        .iter()
        .chain(&bytes[5..7])
        .chain(&bytes[8..10])
        .chain(&bytes[11..13])
        .chain(&bytes[14..16])
        .chain(&bytes[17..19])
        .all(u8::is_ascii_digit)
    {
        return false;
    }
    let year = parse_decimal(&bytes[0..4]);
    let month = parse_decimal(&bytes[5..7]);
    let day = parse_decimal(&bytes[8..10]);
    let hour = parse_decimal(&bytes[11..13]);
    let minute = parse_decimal(&bytes[14..16]);
    let second = parse_decimal(&bytes[17..19]);
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return false;
    }
    let suffix = &bytes[19..];
    if suffix == b"Z" {
        return true;
    }
    let (fraction, offset) = if suffix.first() == Some(&b'.') {
        if suffix.last() == Some(&b'Z') {
            (&suffix[1..suffix.len() - 1], &suffix[suffix.len() - 1..])
        } else {
            let Some(offset_start) = suffix.iter().position(|byte| matches!(*byte, b'+' | b'-'))
            else {
                return false;
            };
            (&suffix[1..offset_start], &suffix[offset_start..])
        }
    } else {
        (&[][..], suffix)
    };
    if suffix.first() == Some(&b'.') && fraction.is_empty() {
        return false;
    }
    if offset == b"Z" {
        return fraction.iter().all(u8::is_ascii_digit);
    }
    if !fraction.iter().all(u8::is_ascii_digit) || offset.len() != 6 {
        return false;
    }
    if !matches!(offset[0], b'+' | b'-')
        || offset[3] != b':'
        || !offset[1..3].iter().all(u8::is_ascii_digit)
        || !offset[4..6].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let offset_hour = parse_decimal(&offset[1..3]);
    let offset_minute = parse_decimal(&offset[4..6]);
    offset_hour <= 23 && offset_minute <= 59
}

/// Converts a validated RFC 3339 timestamp to UTC epoch seconds. Fractional seconds are ignored
/// for retention comparisons, while the explicit timezone offset is applied before comparison.
pub(crate) fn timestamp_epoch_seconds(value: &str) -> Option<i64> {
    if !valid_timestamp(value) {
        return None;
    }
    let bytes = value.as_bytes();
    let year = i64::from(parse_decimal(&bytes[0..4]));
    let month = i64::from(parse_decimal(&bytes[5..7]));
    let day = i64::from(parse_decimal(&bytes[8..10]));
    let hour = i64::from(parse_decimal(&bytes[11..13]));
    let minute = i64::from(parse_decimal(&bytes[14..16]));
    let second = i64::from(parse_decimal(&bytes[17..19]).min(59));
    let days = days_from_civil(year, month, day)?;
    let local = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3_600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    let suffix = &bytes[19..];
    let offset = if suffix == b"Z" {
        0_i64
    } else {
        let sign_index = suffix
            .iter()
            .position(|byte| matches!(*byte, b'+' | b'-'))?;
        let offset = &suffix[sign_index..];
        if offset.len() != 6 {
            return None;
        }
        let hours = i64::from(parse_decimal(&offset[1..3]));
        let minutes = i64::from(parse_decimal(&offset[4..6]));
        let seconds = hours
            .checked_mul(3_600)?
            .checked_add(minutes.checked_mul(60)?)?;
        if offset[0] == b'-' { -seconds } else { seconds }
    };
    local.checked_sub(offset)
}

pub(crate) fn timestamp_expired(value: &str) -> bool {
    let Some(expiry) = timestamp_epoch_seconds(value) else {
        return true;
    };
    let Some(now) = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
    else {
        return false;
    };
    now >= expiry
}

pub(crate) fn timestamp_within_future(value: &str, maximum_seconds: u64) -> bool {
    let Some(expiry) = timestamp_epoch_seconds(value) else {
        return false;
    };
    let Some(now) = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
    else {
        return false;
    };
    let Ok(maximum) = i64::try_from(maximum_seconds) else {
        return false;
    };
    expiry >= now && expiry.saturating_sub(now) <= maximum
}

fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    // Proleptic Gregorian conversion from Howard Hinnant's public-domain algorithm.
    let adjusted_year = year.checked_sub(i64::from(month <= 2))?;
    let era = if adjusted_year >= 0 {
        adjusted_year / 400
    } else {
        (adjusted_year - 399) / 400
    };
    let year_of_era = adjusted_year.checked_sub(era.checked_mul(400)?)?;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153_i64.checked_mul(month_prime)? + 2) / 5 + day - 1;
    let day_of_era = year_of_era
        .checked_mul(365)?
        .checked_add(year_of_era / 4)?
        .checked_sub(year_of_era / 100)?
        .checked_add(day_of_year)?;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}

fn parse_decimal(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0_u32, |value, byte| value * 10 + u32::from(byte - b'0'))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn unique_ids(values: &[String]) -> bool {
    values.iter().all(|value| valid_id(value))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

pub(crate) fn digest(bytes: impl AsRef<[u8]>) -> String {
    let bytes = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        timestamp_epoch_seconds, timestamp_expired, timestamp_within_future, valid_timestamp,
    };

    #[test]
    fn timestamps_are_calendar_and_zone_checked() {
        assert!(valid_timestamp("2099-01-01T00:00:00Z"));
        assert!(valid_timestamp("2024-02-29T23:59:60.123+05:30"));
        assert!(!valid_timestamp("2099-02-29T00:00:00Z"));
        assert!(!valid_timestamp("2099-13-01T00:00:00Z"));
        assert!(!valid_timestamp("2099-01-01T24:00:00Z"));
        assert!(!valid_timestamp("2099-01-01T00:00:00.1"));
    }

    #[test]
    fn timestamps_convert_offsets_for_retention() {
        assert_eq!(timestamp_epoch_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            timestamp_epoch_seconds("1970-01-01T01:00:00+01:00"),
            Some(0)
        );
        assert!(
            timestamp_epoch_seconds("1970-01-01T00:00:01Z")
                > timestamp_epoch_seconds("1970-01-01T00:00:00Z")
        );
        assert!(timestamp_expired("2000-01-01T00:00:00Z"));
        assert!(!timestamp_expired("2099-01-01T00:00:00Z"));
        assert!(timestamp_within_future(
            "2099-01-01T00:00:00Z",
            i64::MAX as u64
        ));
        assert!(!timestamp_within_future("2099-01-01T00:00:00Z", 86_400));
    }
}
