// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::Digest;

use super::super::wire;

pub(super) fn operation_record(
    payload: &Value,
    operation: &sts2_harness::StoredOperation,
) -> Result<String, String> {
    let value = payload
        .get("operation")
        .filter(|value| !value.is_null())
        .ok_or_else(|| String::from("recovery response omitted the operation record"))?;
    let object = value
        .as_object()
        .ok_or_else(|| String::from("recovery operation record is not an object"))?;
    if object.get("operation_id").and_then(Value::as_str)
        != Some(operation.intent.operation_id.as_str())
        || object.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
    {
        return Err(String::from(
            "recovery operation record does not match the original payload digest",
        ));
    }
    let boundary = object
        .get("expected_boundary")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("recovery operation record omitted expected boundary"))?;
    if boundary.get("state_id").and_then(Value::as_str) != Some(operation.intent.state_id.as_str())
        || boundary.get("generation").and_then(Value::as_u64) != Some(operation.intent.generation)
        || boundary.get("catalog_digest").and_then(Value::as_str)
            != operation.intent.catalog_digest.as_deref()
    {
        return Err(String::from(
            "recovery operation record does not match the original boundary",
        ));
    }
    let action = object
        .get("action")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("recovery operation record omitted action identity"))?;
    if action.get("schema_digest").and_then(Value::as_str) != Some(wire::RUNTIME_V3_SCHEMA_DIGEST)
        || action.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
    {
        return Err(String::from(
            "recovery operation record contains an invalid canonical action identity",
        ));
    }
    let encoded = action
        .get("canonical_json_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("recovery operation record omitted canonical action bytes"))?;
    let decoded = super::base64::decode(encoded).ok_or_else(|| {
        String::from("recovery operation record contains invalid canonical action bytes")
    })?;
    let retained = operation.intent.action_payload.as_deref().ok_or_else(|| {
        String::from("durable operation has no canonical action bytes for recovery")
    })?;
    if decoded.as_slice() != retained
        || format!("{:x}", sha2::Sha256::digest(&decoded)) != operation.intent.payload_digest
    {
        return Err(String::from(
            "recovery operation record canonical action bytes do not match the original digest",
        ));
    }
    object
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| String::from("recovery operation record omitted state"))
}
