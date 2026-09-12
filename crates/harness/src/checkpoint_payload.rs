// SPDX-License-Identifier: MIT

//! Consumer verification of the frozen restricted canonical envelope.

use std::sync::OnceLock;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::VerificationFailure;

pub(super) fn verify(bytes: &[u8], manifest: &Value) -> Result<(), VerificationFailure> {
    let payload: Value =
        serde_json::from_slice(bytes).map_err(|_| VerificationFailure::IntegrityFailure)?;
    // ASCII property names make serde's sorted object order equal to the contract's
    // UTF-16 order. Byte equality also rejects duplicates, lexical number changes,
    // BOMs, alternate escaping, whitespace and a trailing newline.
    if !is_canonical(&payload, bytes) || !valid_envelope(&payload) {
        return Err(VerificationFailure::IntegrityFailure);
    }
    if payload["boundary"] != manifest["boundary"] {
        return Err(VerificationFailure::IntegrityFailure);
    }
    let compatibility = serde_json::to_vec(&payload["compatibility"])
        .map_err(|_| VerificationFailure::IntegrityFailure)?;
    let hex: String = Sha256::digest(compatibility)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if manifest["compatibility_digest"].as_str() != Some(format!("sha256:{hex}").as_str()) {
        return Err(VerificationFailure::IncompatibleProfile);
    }
    if payload["compatibility"]["coverage_contract_digest"] != manifest["coverage_contract_digest"]
    {
        return Err(VerificationFailure::CoverageIncomplete);
    }
    Ok(())
}

pub(super) fn is_canonical(value: &Value, bytes: &[u8]) -> bool {
    restricted(value, 0) && serde_json::to_vec(value).ok().as_deref() == Some(bytes)
}

fn restricted(value: &Value, depth: usize) -> bool {
    match value {
        Value::Number(number) => number
            .as_i64()
            .is_some_and(|n| (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&n)),
        Value::Object(fields) => {
            depth < 64
                && fields.iter().all(|(key, value)| {
                    let mut bytes = key.bytes();
                    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
                        && bytes.all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                        })
                        && restricted(value, depth + 1)
                })
        }
        Value::Array(values) => {
            depth < 64 && values.iter().all(|value| restricted(value, depth + 1))
        }
        _ => true,
    }
}

fn valid_envelope(payload: &Value) -> bool {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(include_str!(
                "../../../protocol-artifact/exact-state-v1/schema.json"
            ))
            .map_err(|error| error.to_string())?;
            jsonschema::validator_for(&schema).map_err(|error| error.to_string())
        })
        .as_ref()
        .is_ok_and(|validator| validator.is_valid(payload))
}
