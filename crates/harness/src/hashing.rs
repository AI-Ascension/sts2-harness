// SPDX-License-Identifier: MIT

//! Lowercase hexadecimal encoding and the crate's SHA-256 digest helper.

use sha2::{Digest as _, Sha256};

/// Encode bytes as lowercase hexadecimal without formatting the digest type.
#[must_use]
pub fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// Return the lowercase hexadecimal SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex_bytes(Sha256::digest(bytes))
}
