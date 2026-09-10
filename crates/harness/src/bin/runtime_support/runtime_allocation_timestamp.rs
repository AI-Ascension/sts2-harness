// SPDX-License-Identifier: MIT

pub(super) fn valid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(20..=30).contains(&bytes.len())
        || !matches!(bytes.get(4), Some(b'-'))
        || !matches!(bytes.get(7), Some(b'-'))
        || !matches!(bytes.get(10), Some(b'T'))
        || !matches!(bytes.get(13), Some(b':'))
        || !matches!(bytes.get(16), Some(b':'))
        || !matches!(bytes.last(), Some(b'Z'))
        || !(bytes[0..4].iter().all(u8::is_ascii_digit)
            && bytes[5..7].iter().all(u8::is_ascii_digit)
            && bytes[8..10].iter().all(u8::is_ascii_digit)
            && bytes[11..13].iter().all(u8::is_ascii_digit)
            && bytes[14..16].iter().all(u8::is_ascii_digit)
            && bytes[17..19].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    if bytes.len() > 20
        && (bytes[19] != b'.'
            || !(1..=9).contains(&(bytes.len() - 21))
            || !bytes[20..bytes.len() - 1].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let year = number_u16(&bytes[0..4]);
    let month = number(&bytes[5..7]);
    let day = number(&bytes[8..10]);
    let hour = number(&bytes[11..13]);
    let minute = number(&bytes[14..16]);
    let second = number(&bytes[17..19]);
    let valid_day = year
        .zip(month)
        .zip(day)
        .is_some_and(|((year, month), day)| {
            let days = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if year % 400 == 0 || year % 4 == 0 && year % 100 != 0 => 29,
                2 => 28,
                _ => 0,
            };
            (1..=days).contains(&day)
        });
    month.is_some_and(|value| (1..=12).contains(&value))
        && valid_day
        && hour.is_some_and(|value| value <= 23)
        && minute.is_some_and(|value| value <= 59)
        && second.is_some_and(|value| value <= 59)
}

fn number(bytes: &[u8]) -> Option<u8> {
    (bytes.len() == 2 && bytes.iter().all(u8::is_ascii_digit))
        .then(|| (bytes[0] - b'0') * 10 + bytes[1] - b'0')
}

fn number_u16(bytes: &[u8]) -> Option<u16> {
    (bytes.len() == 4 && bytes.iter().all(u8::is_ascii_digit)).then(|| {
        u16::from(bytes[0] - b'0') * 1_000
            + u16::from(bytes[1] - b'0') * 100
            + u16::from(bytes[2] - b'0') * 10
            + u16::from(bytes[3] - b'0')
    })
}
