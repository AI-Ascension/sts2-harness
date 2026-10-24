// SPDX-License-Identifier: MIT

use sha2::{Digest, Sha256};

use super::super::seed_key::{SeedKeyError, hex_lower};

pub(super) fn hash_framed(parts: &[&[u8]]) -> Result<String, SeedKeyError> {
    let mut digest = Sha256::new();
    for part in parts {
        let length = u32::try_from(part.len()).map_err(|_| SeedKeyError::MessageTooLarge)?;
        digest.update(length.to_be_bytes());
        digest.update(part);
    }
    Ok(hex_lower(&digest.finalize()))
}

pub(super) fn parse_digest(value: &str) -> Result<[u8; 32], SeedKeyError> {
    if value.len() != 64 {
        return Err(SeedKeyError::InvalidIdentity);
    }
    let mut bytes = [0_u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let high = nibble(value.as_bytes()[index * 2]).ok_or(SeedKeyError::InvalidIdentity)?;
        let low = nibble(value.as_bytes()[index * 2 + 1]).ok_or(SeedKeyError::InvalidIdentity)?;
        *slot = (high << 4) | low;
    }
    Ok(bytes)
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
