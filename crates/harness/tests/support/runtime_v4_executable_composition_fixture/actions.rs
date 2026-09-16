// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn unknown_action(request: &Value) -> Result<(u16, Value), String> {
    if request["action"]["action_id"] != ACTION_ID {
        return Err(String::from(
            "expert action was not the host-generated potion action",
        ));
    }
    let mut response = golden_action()?;
    for (field, value) in [
        ("correlation_id", request["correlation_id"].clone()),
        ("instance_id", json!(INSTANCE_ID)),
        ("session_id", json!(SESSION_ID)),
        ("lease_id", json!(LEASE_ID)),
        ("lease_epoch", json!(LEASE_EPOCH)),
        ("generation", json!(7)),
        ("state_id", json!("live:7")),
        ("operation_id", request["operation_id"].clone()),
        ("kind", json!("action_response")),
        ("action", request["action"].clone()),
        ("status", json!("unknown")),
        ("observation", Value::Null),
        ("transition", Value::Null),
        ("error_code", json!("transport_timeout")),
    ] {
        response[field] = value;
    }
    Ok((503, response))
}

pub(super) fn settled_action(
    path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(u16, Value), String> {
    let operation_id = operation_id(path)?;
    let mut response = golden_action()?;
    for (field, value) in [
        (
            "correlation_id",
            json!(
                headers
                    .get("x-sts2-correlation-id")
                    .cloned()
                    .unwrap_or_default()
            ),
        ),
        ("instance_id", json!(INSTANCE_ID)),
        ("session_id", json!(SESSION_ID)),
        ("lease_id", json!(LEASE_ID)),
        ("lease_epoch", json!(LEASE_EPOCH)),
        ("generation", json!(8)),
        ("state_id", json!("live:8")),
        ("operation_id", json!(operation_id)),
        ("kind", json!("action_response")),
        ("action", action_reference()),
        ("status", json!("settled")),
        ("observation", super::expert_observation("live:8", 8, true)),
        (
            "transition",
            json!({"kind":"potion_use_settled","before_generation":7,
                   "after_generation":8,"potion_id":"potion:fire","removed":true}),
        ),
        ("error_code", Value::Null),
    ] {
        response[field] = value;
    }
    Ok((200, response))
}

pub(super) fn unknown_operation(
    path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(u16, Value), String> {
    let operation_id = operation_id(path)?;
    let mut response = golden_action()?;
    for (field, value) in [
        (
            "correlation_id",
            json!(
                headers
                    .get("x-sts2-correlation-id")
                    .cloned()
                    .unwrap_or_default()
            ),
        ),
        ("instance_id", json!(INSTANCE_ID)),
        ("session_id", json!(SESSION_ID)),
        ("lease_id", json!(LEASE_ID)),
        ("lease_epoch", json!(LEASE_EPOCH)),
        ("generation", json!(7)),
        ("state_id", json!("live:7")),
        ("operation_id", json!(operation_id)),
        ("kind", json!("action_response")),
        ("action", action_reference()),
        ("status", json!("unknown")),
        ("observation", Value::Null),
        ("transition", Value::Null),
        ("error_code", json!("transport_timeout")),
    ] {
        response[field] = value;
    }
    Ok((503, response))
}

fn operation_id(path: &str) -> Result<&str, String> {
    path.strip_prefix("/api/v4/runtime/expert-actions/")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| String::from("reconcile operation identity is missing"))
}

fn action_reference() -> Value {
    json!({"action_id":ACTION_ID,
           "action":{"kind":"use_potion","potion_id":"potion:fire","target_id":"enemy:1"}})
}

fn golden_action() -> Result<Value, String> {
    serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
    )))
    .map_err(|error| error.to_string())
}
