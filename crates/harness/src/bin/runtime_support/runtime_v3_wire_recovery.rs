// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::mcp::McpProcess;

pub(in super::super) fn initialize_recovery_mcp(mcp: &mut McpProcess) -> Result<(), String> {
    let initialize = super::rpc_call(
        mcp,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "sts2-harness-recovery", "version": "0.0.0"}
        }),
    )?;
    if initialize.get("result").is_none() {
        return Err(String::from("recovery MCP initialize omitted result"));
    }
    let catalog = super::rpc_call(mcp, 2, "tools/list", json!({}))?;
    validate_catalog(&catalog)
}

pub(in super::super) fn recovery_call(
    mcp: &mut McpProcess,
    id: u64,
    mcp_session_id: &str,
    name: &str,
    expected_kind: &str,
    payload: Value,
) -> Result<Value, String> {
    let response = super::rpc_call(
        mcp,
        id,
        "tools/call",
        json!({
            "name": name,
            "arguments": {"mcp_session_id": mcp_session_id, "payload": payload}
        }),
    )?;
    let text = response
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("recovery MCP tool {name} omitted text content"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("recovery MCP tool {name} returned non-JSON content: {error}"))?;
    if value.get("contract").and_then(Value::as_str) != Some("watchdog-recovery-v1")
        || value.get("schema_digest").and_then(Value::as_str)
            != Some(sts2_harness::RECOVERY_SCHEMA_DIGEST)
        || value
            .get("correlation_id")
            .and_then(Value::as_str)
            .is_none()
        || value.get("kind").and_then(Value::as_str) != Some(expected_kind)
        || !value.get("payload").is_some_and(Value::is_object)
    {
        return Err(format!(
            "recovery MCP tool {name} returned an invalid sideband envelope"
        ));
    }
    Ok(value)
}

pub(super) fn has_recovery_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| {
            value["contract"] == "watchdog-recovery-v1"
                && value["schema_digest"].as_str() == Some(sts2_harness::RECOVERY_SCHEMA_DIGEST)
        })
}

fn validate_catalog(response: &Value) -> Result<(), String> {
    let result = response
        .get("result")
        .ok_or_else(|| String::from("recovery MCP tools/list omitted result"))?;
    if result.get("revision").and_then(Value::as_str) != Some("watchdog-recovery-v1-mcp") {
        return Err(String::from(
            "recovery MCP catalog is not watchdog-recovery-v1-mcp",
        ));
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("recovery MCP catalog omitted tools"))?;
    let expected = [
        "watchdog.bootstrap",
        "watchdog.host_fence",
        "watchdog.lease_acquire",
        "watchdog.lease_renew",
        "watchdog.lease_revoke",
        "watchdog.operation_intent",
        "watchdog.operation_dispatch",
        "watchdog.operation_lookup",
        "watchdog.operation_reconcile",
    ];
    if tools.len() != expected.len()
        || tools
            .iter()
            .zip(expected)
            .any(|(tool, expected)| tool.get("name").and_then(Value::as_str) != Some(expected))
    {
        return Err(String::from(
            "recovery MCP catalog does not expose the exact sideband tool surface",
        ));
    }
    Ok(())
}
