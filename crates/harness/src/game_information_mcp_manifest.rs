// SPDX-License-Identifier: MIT
//! Fixed MCP manifest mapping, with no model-controlled owner or content selector.
use super::*;

pub fn call_content_manifest_mcp<F>(
    context: &LookupMcpContext,
    id: u64,
    rpc: F,
) -> Result<Vec<u8>, LookupError>
where
    F: FnOnce(u64, Value) -> Result<Value, LookupError>,
{
    if id == 0 {
        return Err(LookupError::Invalid);
    }
    let response = rpc(
        id,
        json!({"name":"sts2.game_information_content_manifest","arguments":{
            "instance_id":context.instance_id,"mcp_session_id":context.mcp_session_id,
            "lease_id":context.lease_id,"lease_epoch":context.lease_epoch
        }}),
    )?;
    content_manifest_response_mcp(&response, id)
}

/// Validate a fixed manifest tool result, including the complete typed refusal arm.
pub fn content_manifest_response_mcp(response: &Value, id: u64) -> Result<Vec<u8>, LookupError> {
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
    let value = super::super::manifest::validation::decode(bytes, &id.to_string())?;
    if response["result"]["isError"].as_bool() != Some(value["kind"] == "error_response")
        || response["result"]
            .get("structuredContent")
            .is_some_and(|structured| structured != &value)
    {
        return Err(LookupError::Invalid);
    }
    Ok(bytes.to_vec())
}
