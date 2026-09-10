// SPDX-License-Identifier: MIT

use serde_json::{Number, Value};

const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn u53(value: &Value) -> bool {
    value.as_number().is_some_and(|number| {
        number.is_u64()
            && number
                .as_u64()
                .is_some_and(|value| value <= MAX_WIRE_INTEGER)
    })
}

pub(super) fn positive_u53(value: &Value) -> Option<u64> {
    value
        .as_number()
        .filter(|number| number.is_u64())
        .and_then(Number::as_u64)
        .filter(|value| (1..=MAX_WIRE_INTEGER).contains(value))
}

pub(super) fn strict_timestamp(value: &str) -> bool {
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
    let year = number(&bytes[0..4]);
    let month = number(&bytes[5..7]);
    let day = number(&bytes[8..10]);
    let hour = number(&bytes[11..13]);
    let minute = number(&bytes[14..16]);
    let second = number(&bytes[17..19]);
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

fn number(bytes: &[u8]) -> Option<u32> {
    bytes.iter().all(u8::is_ascii_digit).then(|| {
        bytes
            .iter()
            .fold(0_u32, |value, byte| value * 10 + u32::from(byte - b'0'))
    })
}
