// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

use super::{AttemptDirectory, PolicyRoot, remove_attempt};

pub(super) fn cleanup_partial(
    roots: &[PolicyRoot],
    attempts: &[AttemptDirectory],
) -> Result<(), &'static str> {
    for (index, attempt) in attempts.iter().enumerate().rev() {
        if let Some(root) = roots.get(index) {
            remove_attempt(root, attempt, u64::MAX)?;
        }
    }
    Ok(())
}

pub(super) fn unix_seconds() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "exo_private_clock")
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

pub(super) fn valid_attempt_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
