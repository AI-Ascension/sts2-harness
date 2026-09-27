// SPDX-License-Identifier: MIT

//! Digest and identifier validation for the control journal.
//!
//! The command digest is what makes an idempotency key replay safe: a stored key is only a hit
//! when the same key, command kind and payload hash to the same value, so a client that reuses a
//! key for a different command is refused rather than silently served the first command's receipt.

use sha2::{Digest, Sha256};

pub(super) fn command_digest(key: &str, kind: &str, payload: &str) -> String {
    Sha256::digest(format!("{kind}\0{key}\0{payload}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn validate_operation_id(operation_id: &str) -> Result<(), String> {
    if operation_id.is_empty() || operation_id.len() > 128 {
        return Err("invalid_operation".to_owned());
    }
    if operation_id
        .bytes()
        .any(|byte| byte <= 0x20 || byte == b'/' || byte == b'\\')
    {
        return Err("invalid_operation".to_owned());
    }
    Ok(())
}
