// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::super::wire;

#[path = "runtime_v3_recovery_evidence.rs"]
mod evidence;
pub(super) use evidence::authoritative_reconcile_state;
#[cfg(test)]
#[path = "runtime_v3_recovery_record_tests.rs"]
mod tests;

pub(super) fn response_state(
    payload: &Value,
    operation: &sts2_harness::StoredOperation,
    original_context: &Value,
) -> Result<Option<String>, String> {
    let status = payload["result"]["status"]
        .as_str()
        .ok_or("recovery response omitted status")?;
    if status == "NOT_FOUND" && payload.get("operation") == Some(&Value::Null) {
        return Ok(None);
    }
    let state = operation_record(payload, operation, original_context)?;
    if state != status {
        return Err(String::from(
            "recovery result disagrees with the operation state",
        ));
    }
    Ok(Some(state))
}

pub(super) fn operation_record(
    payload: &Value,
    operation: &sts2_harness::StoredOperation,
    original_context: &Value,
) -> Result<String, String> {
    let object = payload
        .get("operation")
        .and_then(Value::as_object)
        .ok_or("recovery response omitted the operation record")?;
    if object.get("operation_id").and_then(Value::as_str)
        != Some(operation.intent.operation_id.as_str())
        || object.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
        || !original_context.is_object()
        || object.get("original_context") != Some(original_context)
    {
        return Err(String::from(
            "recovery operation does not match the original identity",
        ));
    }
    validate_boundary(object, operation)?;
    validate_action(object, operation)?;
    object
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| String::from("recovery operation record omitted state"))
}

fn validate_boundary(
    object: &Map<String, Value>,
    operation: &sts2_harness::StoredOperation,
) -> Result<(), String> {
    let boundary = object
        .get("expected_boundary")
        .and_then(Value::as_object)
        .ok_or("recovery operation record omitted expected boundary")?;
    if boundary.get("state_id").and_then(Value::as_str) != Some(operation.intent.state_id.as_str())
        || boundary.get("generation").and_then(Value::as_u64) != Some(operation.intent.generation)
        || boundary.get("catalog_digest").and_then(Value::as_str)
            != operation.intent.catalog_digest.as_deref()
    {
        return Err(String::from(
            "recovery operation record does not match the original boundary",
        ));
    }
    Ok(())
}

fn validate_action(
    object: &Map<String, Value>,
    operation: &sts2_harness::StoredOperation,
) -> Result<(), String> {
    let action = object
        .get("action")
        .and_then(Value::as_object)
        .ok_or("recovery operation record omitted action identity")?;
    if action.get("schema_digest").and_then(Value::as_str) != Some(wire::RUNTIME_V3_SCHEMA_DIGEST)
        || action.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
    {
        return Err(String::from(
            "recovery operation record contains an invalid canonical action identity",
        ));
    }
    let decoded = action
        .get("canonical_json_b64")
        .and_then(Value::as_str)
        .and_then(wire::decode_recovery_action)
        .ok_or("recovery operation record contains invalid canonical action bytes")?;
    let retained = operation
        .intent
        .action_payload
        .as_deref()
        .ok_or("durable operation has no canonical action bytes for recovery")?;
    if decoded.as_slice() != retained
        || sts2_harness::sha256_hex(&decoded) != operation.intent.payload_digest
    {
        return Err(String::from(
            "recovery operation record canonical action bytes do not match the original digest",
        ));
    }
    Ok(())
}
