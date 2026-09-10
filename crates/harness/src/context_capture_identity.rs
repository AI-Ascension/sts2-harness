// SPDX-License-Identifier: MIT

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_GENERATED_ATTEMPT: AtomicU64 = AtomicU64::new(0);

pub fn generated_capture_attempt_id(prefix: &str) -> String {
    let serial = NEXT_GENERATED_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-attempt-{}-{nanos}-{serial}", std::process::id())
}

pub(crate) fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphanumeric() || (index > 0 && "._:-".contains(character))
        })
}

pub(crate) fn snapshot_id(
    execution_id: &str,
    attempt_id: Option<&str>,
    boundary: super::CaptureBoundary,
) -> String {
    canonical_id(
        "snapshot",
        &[
            execution_id,
            attempt_id.unwrap_or("<unavailable>"),
            boundary.as_str(),
        ],
    )
}

pub(crate) fn lifecycle_id(
    execution_id: &str,
    attempt_id: Option<&str>,
    boundary: super::CaptureBoundary,
    state: super::TransportState,
    code: Option<&str>,
) -> String {
    canonical_id(
        "event",
        &[
            execution_id,
            attempt_id.unwrap_or("<unavailable>"),
            boundary.as_str(),
            state.as_str(),
            code.unwrap_or("<none>"),
        ],
    )
}

fn canonical_id(prefix: &str, fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ascension.context-capture-id.v1\0");
    for field in fields {
        hasher.update(((*field).len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    format!("{prefix}-{hex}")
}
