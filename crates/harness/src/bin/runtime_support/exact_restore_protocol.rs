// SPDX-License-Identifier: MIT

use std::sync::OnceLock;

use serde_json::{Value, json};

use super::{MAX_FRAME_BYTES, WRAPPER_CONTRACT, WRAPPER_SCHEMA_DIGEST};

pub(crate) const MAX_CHUNK_BYTES: usize = 8 * 1024;

pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[((first & 0x03) << 4 | second.unwrap_or(0) >> 4) as usize] as char);
        encoded.push(match second {
            Some(second) => {
                TABLE[((second & 0x0f) << 2 | third.unwrap_or(0) >> 6) as usize] as char
            }
            None => '=',
        });
        encoded.push(match third {
            Some(third) => TABLE[(third & 0x3f) as usize] as char,
            None => '=',
        });
    }
    encoded
}

pub(crate) fn neutral_validator() -> Result<&'static jsonschema::Validator, String> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(include_str!(
                "../../../../../contract-artifact/exact-restore-v1/schema.json"
            ))
            .map_err(|error| error.to_string())?;
            jsonschema::validator_for(&schema).map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn wrapper_validator() -> Result<&'static jsonschema::Validator, String> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(include_str!(
                "../../../../../contract-artifact/exact-restore-gateway-v1/schema.json"
            ))
            .map_err(|error| error.to_string())?;
            jsonschema::validator_for(&schema).map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn canonical_bytes(value: &Value) -> Result<Vec<u8>, String> {
    sts2_harness::workflow::canonical_json_bytes(value)
        .map_err(|error| format!("exact-restore frame is not canonicalizable: {error}"))
}

pub(crate) fn valid_schema(validator: &jsonschema::Validator, value: &Value) -> bool {
    validator.is_valid(value)
}

pub(crate) fn wrapper_request(frame: &Value, principal_id: &str) -> Result<Value, String> {
    let wrapper = json!({
        "contract": WRAPPER_CONTRACT,
        "schema_digest": WRAPPER_SCHEMA_DIGEST,
        "message_id": frame["message_id"],
        "correlation_id": frame["correlation_id"],
        "actor": {"principal_id": principal_id, "role": "harness"},
        "auth": {
            "principal_id": principal_id,
            "capability": "exact_restore",
            "proof": null,
        },
        "kind": "exact_restore_request",
        "payload": {"frame": frame},
    });
    let bytes = canonical_bytes(&wrapper)?;
    if bytes.len() > MAX_FRAME_BYTES || !valid_schema(wrapper_validator()?, &wrapper) {
        return Err(String::from(
            "exact-restore consumer wrapper exceeds its closed profile schema or byte bound",
        ));
    }
    Ok(wrapper)
}
