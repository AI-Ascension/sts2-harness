// SPDX-License-Identifier: MIT
use super::*;
use serde_json::{Map, json};

/// Calls the additive bootstrap MCP tool through the existing JSON-RPC process.
/// The MCP owner supplies transport identity; the protocol envelope remains the
/// authenticated payload consumed by the gateway/mod producer.
pub fn call_live_observation_bootstrap_mcp<F>(
    context: &LookupMcpContext,
    request: &Value,
    rpc: F,
) -> Result<Vec<u8>, LookupError>
where
    F: FnOnce(u64, Value) -> Result<Value, LookupError>,
{
    crate::game_information_binding::game_information_bootstrap::validate_request(request)
        .map_err(|error| match error {
            crate::game_information_binding::game_information_bootstrap::BootstrapError::Bounds => {
                LookupError::Bounds
            }
            crate::game_information_binding::game_information_bootstrap::BootstrapError::Scope => {
                LookupError::Scope
            }
            _ => LookupError::Invalid,
        })?;
    let id = request["correlation_id"]
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|id| *id > 0)
        .ok_or(LookupError::Invalid)?;
    let selector = request["selector"]
        .as_object()
        .ok_or(LookupError::Invalid)?;
    let scope = request["scope"].as_object().ok_or(LookupError::Invalid)?;
    let mut arguments = Map::new();
    for (key, value) in [
        ("instance_id", &context.instance_id),
        ("mcp_session_id", &context.mcp_session_id),
        ("lease_id", &context.lease_id),
    ] {
        arguments.insert(key.to_owned(), json!(value));
    }
    arguments.insert("lease_epoch".to_owned(), json!(context.lease_epoch));
    arguments.insert(
        "run_id".to_owned(),
        scope.get("run_id").cloned().ok_or(LookupError::Invalid)?,
    );
    arguments.insert(
        "authority_epoch".to_owned(),
        scope
            .get("authority_epoch")
            .cloned()
            .ok_or(LookupError::Invalid)?,
    );
    arguments.insert(
        "content_manifest_id".to_owned(),
        scope
            .get("content_manifest_id")
            .cloned()
            .ok_or(LookupError::Invalid)?,
    );
    arguments.insert(
        "locale".to_owned(),
        scope.get("locale").cloned().ok_or(LookupError::Invalid)?,
    );
    arguments.insert(
        "definition_ref".to_owned(),
        selector
            .get("definition_ref")
            .cloned()
            .ok_or(LookupError::Invalid)?,
    );
    arguments.insert(
        "instance_ref".to_owned(),
        selector
            .get("instance_ref")
            .cloned()
            .ok_or(LookupError::Invalid)?,
    );
    let limits = request["limits"].as_object().ok_or(LookupError::Invalid)?;
    for key in [
        "max_visible_entities",
        "max_item_bytes",
        "max_message_bytes",
    ] {
        arguments.insert(
            key.to_owned(),
            limits.get(key).cloned().ok_or(LookupError::Invalid)?,
        );
    }
    let response = rpc(
        id,
        json!({
            "name": "sts2.game_information.live_observation_bootstrap",
            "arguments": arguments
        }),
    )?;
    if response["jsonrpc"] != "2.0" || response["id"] != id || !response["error"].is_null() {
        return Err(LookupError::Transport);
    }
    let contents = response["result"]["content"]
        .as_array()
        .ok_or(LookupError::Invalid)?;
    if contents.len() != 1 || contents[0]["type"] != "text" {
        return Err(LookupError::Invalid);
    }
    let bytes = contents[0]["text"]
        .as_str()
        .ok_or(LookupError::Invalid)?
        .as_bytes();
    if bytes.len() > crate::game_information_binding::game_information_bootstrap::MAX_MESSAGE_BYTES
    {
        return Err(LookupError::Bounds);
    }
    if response["result"]["isError"] == true {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| LookupError::Invalid)?;
        if value["kind"] != "error_response"
            || value["protocol_version"]
                != crate::game_information_binding::game_information_bootstrap::PROFILE
            || value["schema_digest"]
                != crate::game_information_binding::game_information_bootstrap::SCHEMA_DIGEST
        {
            return Err(LookupError::Invalid);
        }
        return match value["error"]["code"].as_str() {
            Some("not_observable" | "unsupported") => Err(LookupError::MissingCapability),
            Some("stale_snapshot" | "reobserve_required") => Err(LookupError::Reobserve),
            Some("scope_denied") => Err(LookupError::Scope),
            Some("bounds") => Err(LookupError::Bounds),
            _ => Err(LookupError::Invalid),
        };
    }
    if response["result"]["isError"] != false {
        return Err(LookupError::Invalid);
    }
    Ok(bytes.to_vec())
}
