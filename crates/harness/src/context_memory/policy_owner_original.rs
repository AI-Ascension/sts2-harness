// SPDX-License-Identifier: MIT

use super::types::{PolicyOwnerError, check_raw};
use crate::context_memory::{MemoryPolicy, MemoryScope, parse_strict_json};
use serde_json::Value;
use std::sync::OnceLock;

pub(super) fn parse_original(
    raw: &[u8],
    scope: &MemoryScope,
) -> Result<MemoryPolicy, PolicyOwnerError> {
    check_raw(raw)?;
    let value: Value = parse_strict_json(raw).map_err(|_| PolicyOwnerError::SchemaInvalid)?;
    validate_original(&value)?;
    if has_float(&value) {
        return Err(PolicyOwnerError::UnsupportedNumericRepresentation);
    }
    // Decode the original again, not a defaulted or numerically coerced projection.
    let policy: MemoryPolicy =
        serde_json::from_slice(raw).map_err(|_| PolicyOwnerError::SchemaInvalid)?;
    policy
        .validate_schema()
        .map_err(|_| PolicyOwnerError::SchemaInvalid)?;
    if policy.scope != *scope {
        return Err(PolicyOwnerError::ScopeMismatch);
    }
    Ok(policy)
}

fn validate_original(value: &Value) -> Result<(), PolicyOwnerError> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, ()>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            // This embedded schema has no external references. The existing dependency also
            // disables default network/file resolution features; callers cannot supply schemas.
            let schema: Value = serde_json::from_str(include_str!(
                "../../../../contracts/context-memory/policy.schema.json"
            ))
            .map_err(|_| ())?;
            jsonschema::validator_for(&schema).map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| PolicyOwnerError::Unavailable)?;
    if !validator.is_valid(value) {
        return Err(PolicyOwnerError::SchemaInvalid);
    }
    Ok(())
}

fn has_float(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.is_f64(),
        Value::Array(values) => values.iter().any(has_float),
        Value::Object(values) => values.values().any(has_float),
        _ => false,
    }
}
