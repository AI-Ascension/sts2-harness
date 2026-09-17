// SPDX-License-Identifier: MIT
use super::*;
use serde_json::{Map, json};

/// Existing MCP port adapter implemented by the runtime owner, never by game text.
pub trait LookupMcpPort {
    /// Returns the next owner-allocated MCP correlation without allowing model-chosen RPC IDs.
    fn information_correlation(&self) -> Result<String, LookupError>;
    fn information_capabilities(&mut self) -> Result<(String, Vec<u8>), LookupError> {
        Err(LookupError::MissingCapability)
    }
    fn call_information(&mut self, tool: &str, request: &Value) -> Result<Vec<u8>, LookupError>;
    /// Additive live-observation bootstrap. Existing MCP-only test ports keep
    /// the safe default and therefore retain static/live-unavailable behavior.
    fn call_live_observation_bootstrap(
        &mut self,
        _request: &Value,
    ) -> Result<Vec<u8>, LookupError> {
        Err(LookupError::MissingCapability)
    }
}

/// Capability read uses only the selected MCP authority context, never a query or mutation.
pub fn call_capabilities_mcp<F>(
    context: &LookupMcpContext,
    id: u64,
    rpc: F,
) -> Result<Vec<u8>, LookupError>
where
    F: FnOnce(u64, Value) -> Result<Value, LookupError>,
{
    let result = rpc(
        id,
        json!({"name":"sts2.game_information_capabilities","arguments":{
            "instance_id":context.instance_id,"mcp_session_id":context.mcp_session_id,
            "lease_id":context.lease_id,"lease_epoch":context.lease_epoch
        }}),
    )?;
    let text = result["result"]["content"]
        .as_array()
        .filter(|items| items.len() == 1)
        .and_then(|items| items[0]["text"].as_str())
        .ok_or(LookupError::MissingCapability)?;
    let value = validation::decode_strict(text.as_bytes())?;
    validation::validate_capabilities(&value)?;
    let correlation = id.to_string();
    if result["jsonrpc"] != "2.0"
        || result["id"] != id
        || result["result"]["isError"] != false
        || result["result"]["content"][0]["type"] != "text"
        || !result["error"].is_null()
        || value["correlation_id"].as_str() != Some(correlation.as_str())
    {
        return Err(LookupError::MissingCapability);
    }
    if result["result"]
        .get("structuredContent")
        .is_some_and(|structured| structured != &value)
    {
        return Err(LookupError::Invalid);
    }
    Ok(text.as_bytes().to_vec())
}

#[derive(Clone, Debug)]
pub struct LookupMcpContext {
    pub instance_id: String,
    pub mcp_session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
}

/// Maps the pinned inert envelope to the accepted fixed MCP tool arguments.
/// The existing MCP port owns framing, transport, timeout and session lifecycle.
pub fn call_lookup_mcp<F>(
    context: &LookupMcpContext,
    tool: &str,
    request: &Value,
    rpc: F,
) -> Result<Vec<u8>, LookupError>
where
    F: FnOnce(u64, Value) -> Result<Value, LookupError>,
{
    validation::validate_request(request)?;
    if tool != admission::tool_name(request)? {
        return Err(LookupError::MissingCapability);
    }
    let id = request["correlation_id"]
        .as_str()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|id| *id > 0)
        .ok_or(LookupError::Invalid)?;
    let canonical_id = id.to_string();
    if request["correlation_id"].as_str() != Some(canonical_id.as_str()) {
        return Err(LookupError::Invalid);
    }
    let arguments = arguments(context, request)?;
    let response = rpc(id, json!({"name":tool,"arguments":arguments}))?;
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
    let source = validation::decode_strict(bytes)?;
    validation::validate_response(request, &source)?;
    if response["result"]["isError"].as_bool() != Some(source["kind"] == "error_response") {
        return Err(LookupError::Invalid);
    }
    if let Some(structured) = response["result"].get("structuredContent")
        && structured != &source
    {
        return Err(LookupError::Invalid);
    }
    Ok(bytes.to_vec())
}

fn arguments(context: &LookupMcpContext, request: &Value) -> Result<Value, LookupError> {
    let query = &request["query"];
    if query["binding"]["mode"] == "live"
        && query["binding"]["instance_ref"]["instance_id"] != context.instance_id
    {
        return Err(LookupError::Scope);
    }
    let mut args = Map::new();
    for (key, value) in [
        ("instance_id", &context.instance_id),
        ("mcp_session_id", &context.mcp_session_id),
        ("lease_id", &context.lease_id),
    ] {
        args.insert(key.to_owned(), json!(value));
    }
    args.insert("lease_epoch".to_owned(), json!(context.lease_epoch));
    for key in ["content_manifest_id", "locale", "visibility_scope"] {
        args.insert(key.to_owned(), query["binding"][key].clone());
    }
    for key in [
        "entity_kind",
        "projection",
        "detail_level",
        "fields",
        "cursor",
    ] {
        args.insert(key.to_owned(), query[key].clone());
    }
    for key in ["page_items", "item_bytes", "page_bytes", "text_bytes"] {
        args.insert(key.to_owned(), query["limits"][key].clone());
    }
    for key in [
        "display_name",
        "namespaced_ids",
        "definition_refs",
        "instance_ids",
    ] {
        args.insert(key.to_owned(), query["filters"][key].clone());
    }
    add_target_arguments(query, &mut args);
    Ok(Value::Object(args))
}

fn add_target_arguments(query: &Value, args: &mut Map<String, Value>) {
    if matches!(
        query["query_kind"].as_str(),
        Some("get" | "detail" | "availability")
    ) {
        args.insert(
            "definition_ref".to_owned(),
            query["target"]["definition_ref"].clone(),
        );
    }
    if matches!(
        query["query_kind"].as_str(),
        Some("detail" | "availability")
    ) {
        args.insert(
            "instance_ref".to_owned(),
            query["binding"]["instance_ref"].clone(),
        );
        args.insert(
            "snapshot_ref".to_owned(),
            query["binding"]["snapshot_ref"].clone(),
        );
        args.insert(
            "parent_observation".to_owned(),
            query["parent_observation"].clone(),
        );
    }
    if query["query_kind"] == "availability" {
        args.insert("mode".to_owned(), query["binding"]["mode"].clone());
    }
}
