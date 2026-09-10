// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::config::RuntimeConfig;
use super::super::mcp::McpProcess;
use super::super::runtime_v3_wire;

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
// MCP carries the complete snapshot as escaped JSON text inside its JSON-RPC content wrapper.
// Keep this profile-specific bound aligned with McpProcess while retaining the snapshot's own
// 256 KiB validation in the consumer parser.
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_GENERATION: u64 = 9_007_199_254_740_991;

/// Starts a short-lived map-profile MCP process, validates its catalog, and reads one snapshot.
/// The process is deliberately scoped to this read so the normal gameplay MCP authority is not
/// silently widened with a second tool surface.
pub(super) fn snapshot(config: &RuntimeConfig, generation: u64) -> Result<Value, String> {
    if generation > MAX_GENERATION {
        return Err(String::from("map generation is outside its bound"));
    }
    let mut mcp = McpProcess::spawn_profile(config, PROFILE)?;
    let result = (|| {
        initialize_mcp(&mut mcp)?;
        let response = runtime_v3_wire::rpc_call(
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
        )
        .map_err(|error| error.to_string())?;
        let text = response
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("MCP map tool omitted JSON content"))?;
        if text.len() > MAX_RESPONSE_BYTES {
            return Err(String::from("MCP map response exceeded its size bound"));
        }
        let value: Value = serde_json::from_str(text)
            .map_err(|_| String::from("MCP map tool returned malformed JSON"))?;
        validate_response(&value, config, generation)?;
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

fn initialize_mcp(mcp: &mut McpProcess) -> Result<(), String> {
    let initialize = runtime_v3_wire::rpc_call(
        mcp,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "sts2-harness-runtime-map-v1", "version": "0.0.0"}
        }),
    )
    .map_err(|error| error.to_string())?;
    if initialize.get("result").is_none() {
        return Err(String::from("MCP map initialize omitted result"));
    }
    let catalog = runtime_v3_wire::rpc_call(mcp, 2, "tools/list", json!({}))
        .map_err(|error| error.to_string())?;
    validate_catalog(&catalog)
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

fn validate_response(value: &Value, config: &RuntimeConfig, generation: u64) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| String::from("MCP map response is not an object"))?;
    let expected = [
        "protocol_version",
        "schema_digest",
        "provenance",
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "lease_epoch",
        "generation",
        "kind",
        "snapshot",
        "timeout",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(String::from(
            "MCP map response envelope has an unexpected shape",
        ));
    }
    if object.get("protocol_version").and_then(Value::as_str) != Some(PROFILE)
        || object.get("schema_digest").and_then(Value::as_str) != Some(SCHEMA_DIGEST)
        || object.get("kind").and_then(Value::as_str) != Some("snapshot_response")
        || object.get("correlation_id").and_then(Value::as_str) != Some("3")
        || object.get("instance_id").and_then(Value::as_str) != Some(config.instance_id.as_str())
        || object.get("session_id").and_then(Value::as_str) != Some(config.session_id.as_str())
        || object.get("lease_id").and_then(Value::as_str) != Some(config.lease_id.as_str())
        || object.get("lease_epoch").and_then(Value::as_u64) != Some(config.lease_epoch)
        || object.get("generation").and_then(Value::as_u64) != Some(generation)
    {
        return Err(String::from(
            "MCP map response identity does not match the request",
        ));
    }
    let provenance = object
        .get("provenance")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("MCP map response provenance is missing"))?;
    if provenance.len() != 3
        || provenance.get("artifact").and_then(Value::as_str)
            != Some("sts2-protocol/runtime-map-v1")
        || provenance.get("source").and_then(Value::as_str)
            != Some("schemas/runtime-map-v1.schema.json")
        || provenance.get("generator").and_then(Value::as_str) != Some("hand-authored")
    {
        return Err(String::from("MCP map response provenance is invalid"));
    }
    let snapshot = object
        .get("snapshot")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("MCP map response snapshot is missing"))?;
    if snapshot.get("generation").and_then(Value::as_u64) != Some(generation) {
        return Err(String::from("MCP map snapshot generation is stale"));
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
}
