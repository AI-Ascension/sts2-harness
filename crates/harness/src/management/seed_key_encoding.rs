// SPDX-License-Identifier: MIT

use super::{MAX_KEY_ID_BYTES, SeedKeyError};

pub(super) fn validate_key_id(value: &str) -> Result<(), SeedKeyError> {
    if value.is_empty()
        || value.len() > MAX_KEY_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(SeedKeyError::InvalidIdentity);
    }
    Ok(())
}

pub(super) fn decode_lower_hex<const N: usize>(encoded: &str) -> Option<[u8; N]> {
    if encoded.len() != N.checked_mul(2)? || !encoded.is_ascii() {
        return None;
    }
    let mut output = [0_u8; N];
    for (index, slot) in output.iter_mut().enumerate() {
        let high = lower_hex_nibble(encoded.as_bytes()[index * 2])?;
        let low = lower_hex_nibble(encoded.as_bytes()[index * 2 + 1])?;
        *slot = (high << 4) | low;
    }
    Some(output)
}

pub(super) fn decode_lower_hex_into<const N: usize>(encoded: &str, output: &mut [u8; N]) -> bool {
    if encoded.len() != N.saturating_mul(2) || !encoded.is_ascii() {
        return false;
    }
    for (index, slot) in output.iter_mut().enumerate() {
        let Some(high) = lower_hex_nibble(encoded.as_bytes()[index * 2]) else {
            return false;
        };
        let Some(low) = lower_hex_nibble(encoded.as_bytes()[index * 2 + 1]) else {
            return false;
        };
        *slot = (high << 4) | low;
    }
    true
}

fn lower_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

pub(in crate::management) fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}
