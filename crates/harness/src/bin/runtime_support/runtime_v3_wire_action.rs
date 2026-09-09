// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::Digest;

pub(in super::super) fn canonical_action_bytes(
    action_id: &str,
    payload: &Value,
) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"action": payload, "action_id": action_id}))
        .map_err(|error| format!("cannot encode canonical runtime-v3 action: {error}"))
}

pub(in super::super) fn canonical_action_digest(
    action_id: &str,
    payload: &Value,
) -> Result<String, String> {
    let bytes = canonical_action_bytes(action_id, payload)?;
    Ok(format!("{:x}", sha2::Sha256::digest(bytes)))
}
