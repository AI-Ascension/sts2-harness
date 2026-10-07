// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::config::RuntimeConfig;
use super::super::mcp::McpProcess;
use super::super::runtime_v3_wire;
use sts2_protocol::{RuntimeMapV1Message, RuntimeMapV1MessageKind, decode_runtime_map_message};

const PROFILE: &str = "runtime-map-v1";
const CATALOG_REVISION: &str = "runtime-map-v1-mcp";
const EXPECTED_TOOLS: [&str; 7] = [
    "sts2.observe",
    "sts2.legal_actions",
    "sts2.dispatch_action",
    "sts2.wait_for_transition",
    "sts2.reobserve",
    "sts2.recover",
    "sts2.map_snapshot",
];
const SCHEMA_DIGEST: &str = "ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b";
const MAX_GENERATION: u64 = 9_007_199_254_740_991;

/// Starts a short-lived map-profile MCP process, validates its catalog, and reads one snapshot.
/// The process is deliberately scoped to this read so the normal gameplay MCP authority is not
/// silently widened with a second tool surface.
pub(super) fn snapshot(config: &RuntimeConfig, generation: u64) -> Result<Value, String> {
    if generation > MAX_GENERATION {
        return Err(String::from("map generation is outside its bound"));
    }
    let deadline = runtime_v3_wire::MapProfileDeadline::start()
        .ok_or_else(|| String::from("MCP map profile deadline is unavailable"))?;
    let mut mcp = McpProcess::spawn_profile_with_deadline(config, PROFILE, deadline.absolute())?;
    let result = (|| {
        initialize_mcp(&mut mcp, &deadline)?;
        let response = runtime_v3_wire::rpc_call_with_deadline(
            &mut mcp,
            3,
            "tools/call",
            json!({
                "name": "sts2.map_snapshot",
                "arguments": {
                    "instance_id": config.instance_id,
                    "mcp_session_id": config.mcp_session_id,
                    "lease_id": config.lease_id,
                    "lease_epoch": config.lease_epoch,
                    "generation": generation
                }
            }),
            &deadline,
        )
        .map_err(|error| error.to_string())?;
        let text = map_tool_text(&response)?;
        let value = decode_response_text(text, config, generation)?;
        if deadline.remaining().is_zero() {
            return Err(String::from(
                "MCP map response validation exceeded the profile deadline",
            ));
        }
        Ok(value)
    })();
    let close = mcp.close();
    match (result, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("MCP map cleanup failed: {error}")),
        (Err(error), Err(close_error)) => {
            Err(format!("{error}; MCP map cleanup failed: {close_error}"))
        }
    }
}

fn map_tool_text(response: &Value) -> Result<&str, String> {
    response
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .filter(|content| content.len() == 1)
        .and_then(|content| content.first())
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("MCP map tool omitted exactly one text content item"))
}

fn initialize_mcp(
    mcp: &mut McpProcess,
    deadline: &runtime_v3_wire::MapProfileDeadline,
) -> Result<(), String> {
    let initialize = runtime_v3_wire::rpc_call_with_deadline(
        mcp,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "sts2-harness-runtime-map-v1", "version": "0.0.0"}
        }),
        deadline,
    )
    .map_err(|error| error.to_string())?;
    if initialize.get("result").is_none() {
        return Err(String::from("MCP map initialize omitted result"));
    }
    let catalog =
        runtime_v3_wire::rpc_call_with_deadline(mcp, 2, "tools/list", json!({}), deadline)
            .map_err(|error| error.to_string())?;
    validate_catalog(&catalog)
}

fn decode_response_text(
    text: &str,
    config: &RuntimeConfig,
    generation: u64,
) -> Result<Value, String> {
    let message = decode_runtime_map_message(text.as_bytes())
        .map_err(|_| String::from("MCP map tool returned an invalid runtime-map-v1 envelope"))?;
    message
        .validate_response()
        .map_err(|_| String::from("MCP map tool returned an invalid runtime-map-v1 response"))?;
    validate_response_identity(&message, config, generation)?;
    serde_json::to_value(message)
        .map_err(|_| String::from("MCP map response could not be encoded for the decision port"))
}

fn validate_catalog(response: &Value) -> Result<(), String> {
    let result = response
        .get("result")
        .ok_or_else(|| String::from("MCP map tools/list omitted result"))?;
    if result.get("revision").and_then(Value::as_str) != Some(CATALOG_REVISION) {
        return Err(String::from("MCP catalog is not runtime-map-v1-mcp"));
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("MCP map catalog omitted tools"))?;
    if tools.len() != EXPECTED_TOOLS.len()
        || tools
            .iter()
            .zip(EXPECTED_TOOLS)
            .any(|(tool, expected)| tool.get("name").and_then(Value::as_str) != Some(expected))
    {
        return Err(String::from(
            "MCP map catalog does not expose the exact seven-tool surface",
        ));
    }
    Ok(())
}

fn validate_response_identity(
    message: &RuntimeMapV1Message,
    config: &RuntimeConfig,
    generation: u64,
) -> Result<(), String> {
    if message.protocol_version.as_str() != PROFILE
        || message.schema_digest.as_str() != SCHEMA_DIGEST
        || message.kind != RuntimeMapV1MessageKind::SnapshotResponse
        || message.correlation_id.as_str() != "3"
        || message.instance_id.as_str() != config.instance_id.as_str()
        // The map response uses the gateway session identity. The MCP session identity remains
        // a separate fixed request argument and is never substituted for this response fence.
        || message.session_id.as_str() != config.session_id.as_str()
        || message.lease_id.as_str() != config.lease_id.as_str()
        || message.lease_epoch != config.lease_epoch
        || message.generation != generation
    {
        return Err(String::from(
            "MCP map response identity does not match the request",
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "runtime_map_tests.rs"]
mod wire_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_requires_ordered_gameplay_tools_and_map_reader() {
        let response = json!({
            "result": {
                "revision": CATALOG_REVISION,
                "tools": EXPECTED_TOOLS.iter().map(|name| json!({"name": name})).collect::<Vec<_>>()
            }
        });
        assert!(validate_catalog(&response).is_ok());
        let mut reordered = response;
        reordered["result"]["tools"][0]["name"] = json!("sts2.map_snapshot");
        assert!(validate_catalog(&reordered).is_err());
    }

    #[test]
    fn map_tool_content_requires_one_text_item() {
        let valid = json!({"result":{"content":[{"type":"text","text":"{}"}]}});
        assert_eq!(map_tool_text(&valid), Ok("{}"));
        let extra = json!({"result":{"content":[
            {"type":"text","text":"{}"}, {"type":"text","text":"{}"}
        ]}});
        assert!(map_tool_text(&extra).is_err());
        let non_text = json!({"result":{"content":[{"type":"image","text":"{}"}]}});
        assert!(map_tool_text(&non_text).is_err());
    }
}
