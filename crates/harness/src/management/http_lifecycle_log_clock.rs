// SPDX-License-Identifier: MIT

//! Timestamp rendering for the management request-lifecycle log.
//!
//! Split out of `http_lifecycle_log` because calendar arithmetic is a distinct
//! concern from request reporting, and because a wrong timestamp is worse than no
//! timestamp: it places an event in the wrong window with total confidence.
//!
//! Everything here is pure and takes its inputs as arguments, so the exact rendering
//! is directly assertable without a real clock.

/// Render UTC `YYYY-MM-DDTHH:MM:SS.mmmZ` from a UNIX epoch split.
///
/// Hand-rolled rather than pulled from a date crate: the dependency set is
/// deliberately small and this is the only calendar arithmetic the harness needs.
/// The civil-date conversion is Howard Hinnant's `civil_from_days`, which is exact
/// across the whole proleptic Gregorian range and needs no lookup table.
pub(super) fn push_utc_timestamp(line: &mut String, seconds: u64, nanos: u32) {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    push_decimal(line, year.unsigned_abs());
    line.push('-');
    push_padded(line, u64::from(month), 2);
    line.push('-');
    push_padded(line, u64::from(day), 2);
    line.push('T');
    push_padded(line, seconds_of_day / 3_600, 2);
    line.push(':');
    push_padded(line, (seconds_of_day / 60) % 60, 2);
    line.push(':');
    push_padded(line, seconds_of_day % 60, 2);
    line.push('.');
    push_padded(line, u64::from(nanos / 1_000_000), 3);
    line.push('Z');
}

/// Days since the UNIX epoch to a proleptic Gregorian `(year, month, day)`.
pub(super) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (year + i64::from(month <= 2), month as u32, day)
}

/// Append `value` in decimal, with no padding and no allocation.
pub(super) fn push_decimal(line: &mut String, value: u64) {
    let mut digits = [0_u8; 20];
    let mut index = digits.len();
    let mut remaining = value;
    loop {
        index -= 1;
        digits[index] = b'0' + u8::try_from(remaining % 10).unwrap_or(0);
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for byte in &digits[index..] {
        line.push(char::from(*byte));
    }
}

/// Append `value` in decimal, zero-padded on the left to `width`.
///
/// Zero-padding uses only the digits actually written, so the full 20-byte buffer's
/// leading zeros are never emitted. Getting this wrong prints a valid-looking but
/// wrong timestamp, which is worse than printing none.
pub(super) fn push_padded(line: &mut String, value: u64, width: usize) {
    let mut digits = [0_u8; 20];
    let mut index = digits.len();
    let mut remaining = value;
    loop {
        index -= 1;
        digits[index] = b'0' + u8::try_from(remaining % 10).unwrap_or(0);
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    let mut written = digits.len() - index;
    while written < width {
        line.push('0');
        written += 1;
    }
    for byte in &digits[index..] {
        line.push(char::from(*byte));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    // The calendar arithmetic is hand-rolled, so it is pinned against known epochs
    // rather than trusted. A wrong timestamp places an event in the wrong window with
    // total confidence, which is worse than emitting none.

    #[test]
    fn the_rendered_timestamp_is_exact_across_known_epochs() {
        let cases = [
            (0_u64, "1970-01-01T00:00:00.123Z"),
            (1_000_000_000, "2001-09-09T01:46:40.123Z"),
            (1_700_000_000, "2023-11-14T22:13:20.123Z"),
            (1_767_225_224, "2025-12-31T23:53:44.123Z"),
            (2_147_483_647, "2038-01-19T03:14:07.123Z"),
        ];
        for (epoch_seconds, expected) in cases {
            let mut line = String::new();
            push_utc_timestamp(&mut line, epoch_seconds, 123_000_000);
            assert_eq!(line, expected, "epoch {epoch_seconds} must render exactly");
        }
    }

    #[test]
    fn a_leap_day_epoch_renders_correctly() {
        // 2024-02-29T12:00:00Z -- the case a naive conversion gets wrong.
        let mut line = String::new();
        push_utc_timestamp(&mut line, 1_709_208_000, 123_000_000);
        assert_eq!(line, "2024-02-29T12:00:00.123Z");
    }

    #[test]
    fn the_century_leap_day_renders_correctly() {
        // 2000-02-29 and 2000-03-01: the adjacent pair that separates a correct
        // proleptic Gregorian conversion from one that is off by a day.
        for (epoch_seconds, expected) in [
            (951_782_400_u64, "2000-02-29T00:00:00.000Z"),
            (951_868_800_u64, "2000-03-01T00:00:00.000Z"),
        ] {
            let mut line = String::new();
            push_utc_timestamp(&mut line, epoch_seconds, 0);
            assert_eq!(line, expected, "epoch {epoch_seconds} must render exactly");
        }
    }

    #[test]
    fn zero_padding_uses_only_the_written_digits() {
        // The 20-byte scratch buffer's leading zeros must never be emitted.
        let mut line = String::new();
        push_padded(&mut line, 0, 2);
        push_padded(&mut line, 7, 3);
        push_padded(&mut line, 42, 2);
        push_padded(&mut line, 1234, 4);
        assert_eq!(line, "00007421234");
    }

    #[test]
    fn decimal_rendering_emits_no_padding() {
        let mut line = String::new();
        push_decimal(&mut line, 0);
        line.push(',');
        push_decimal(&mut line, 1_767_225_224);
        assert_eq!(line, "0,1767225224");
    }
}
