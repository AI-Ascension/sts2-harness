// SPDX-License-Identifier: MIT

use super::*;

/// Echo the identity carried by the request envelope or its fenced headers.
///
/// The gateway compares every echoed identity field against the request it
/// forwarded, so the action responses have to answer as the negotiated
/// deployment rather than as the fixture default. `expert-action` arrives with
/// the identity in the body and its reconciliation arrives with the identity in
/// the headers, so both sources are read.
fn echoed(
    request: Option<&Value>,
    headers: &BTreeMap<String, String>,
) -> [(&'static str, Value); 4] {
    let body = |name: &str| request.and_then(|value| value.get(name)).cloned();
    let header = |name: &str| json!(headers.get(name).cloned().unwrap_or_default());
    [
        (
            "instance_id",
            body("instance_id").unwrap_or_else(|| header("x-sts2-instance-id")),
        ),
        (
            "session_id",
            body("session_id").unwrap_or_else(|| header("x-sts2-session-id")),
        ),
        (
            "lease_id",
            body("lease_id").unwrap_or_else(|| header("x-sts2-lease-id")),
        ),
        (
            "lease_epoch",
            body("lease_epoch").unwrap_or_else(|| header_epoch(headers)),
        ),
    ]
}

fn header_epoch(headers: &BTreeMap<String, String>) -> Value {
    json!(
        headers
            .get("x-sts2-lease-epoch")
            .and_then(|epoch| epoch.parse::<u64>().ok())
            .unwrap_or(0)
    )
}

pub(super) fn unknown_action(request: &Value) -> Result<(u16, Value), String> {
    if request["action"]["action_id"] != ACTION_ID {
        return Err(String::from(
            "expert action was not the host-generated potion action",
        ));
    }
    let mut response = golden_action()?;
    let identity = echoed(Some(request), &BTreeMap::new());
    for (field, value) in identity.into_iter().chain([
        ("correlation_id", request["correlation_id"].clone()),
        ("generation", json!(7)),
        ("state_id", json!("live:7")),
        ("operation_id", request["operation_id"].clone()),
        ("kind", json!("action_response")),
        ("action", request["action"].clone()),
        ("status", json!("unknown")),
        ("observation", Value::Null),
        ("transition", Value::Null),
        ("error_code", json!("transport_timeout")),
    ]) {
        response[field] = value;
    }
    Ok((503, response))
}

pub(super) fn accepted_action(request: &Value) -> Result<(u16, Value), String> {
    if request["action"]["action_id"] != ACTION_ID {
        return Err(String::from(
            "expert action was not the host-generated potion action",
        ));
    }
    let mut response = golden_action()?;
    let identity = echoed(Some(request), &BTreeMap::new());
    for (field, value) in identity.into_iter().chain([
        ("correlation_id", request["correlation_id"].clone()),
        ("generation", json!(7)),
        ("state_id", json!("live:7")),
        ("operation_id", request["operation_id"].clone()),
        ("kind", json!("action_response")),
        ("action", request["action"].clone()),
        ("status", json!("accepted")),
        ("observation", Value::Null),
        ("transition", Value::Null),
        ("error_code", Value::Null),
    ]) {
        response[field] = value;
    }
    Ok((200, response))
}

pub(super) fn settled_action(
    path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(u16, Value), String> {
    let operation_id = operation_id(path)?;
    let mut response = golden_action()?;
    let identity = echoed(None, headers);
    for (field, value) in identity.into_iter().chain([
        (
            "correlation_id",
            json!(
                headers
                    .get("x-sts2-correlation-id")
                    .cloned()
                    .unwrap_or_default()
            ),
        ),
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
    ]) {
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
    let identity = echoed(None, headers);
    for (field, value) in identity.into_iter().chain([
        (
            "correlation_id",
            json!(
                headers
                    .get("x-sts2-correlation-id")
                    .cloned()
                    .unwrap_or_default()
            ),
        ),
        ("generation", json!(7)),
        ("state_id", json!("live:7")),
        ("operation_id", json!(operation_id)),
        ("kind", json!("action_response")),
        ("action", action_reference()),
        ("status", json!("unknown")),
        ("observation", Value::Null),
        ("transition", Value::Null),
        ("error_code", json!("transport_timeout")),
    ]) {
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
