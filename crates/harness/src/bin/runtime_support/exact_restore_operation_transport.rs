// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::{
    MAX_FRAME_BYTES, NEUTRAL_CONTRACT, NEUTRAL_SCHEMA_DIGEST, canonical_bytes, neutral_validator,
    valid_schema, wrapper_request,
};
use super::uncertain;
use crate::runtime_support::{config::RuntimeConfig, mcp_process::McpProcess};

pub(crate) fn exchange(
    process: &mut Option<McpProcess>,
    rpc_id: &mut u64,
    config: &RuntimeConfig,
    request: &Value,
    tool: &str,
) -> Result<Value, super::super::ExactRestoreError> {
    ensure_profile(process, config).map_err(|message| {
        uncertain(format!("exact-restore MCP profile unavailable: {message}"))
    })?;
    let request_id = *rpc_id;
    *rpc_id = rpc_id
        .checked_add(1)
        .ok_or_else(|| uncertain("exact-restore RPC identity exhausted"))?;
    let wrapper = wrapper_request(request, &config.caller_id).map_err(super::not_started)?;
    let request_digest = format!(
        "sha256:{}",
        sts2_harness::sha256_hex(&canonical_bytes(request).map_err(super::not_started)?)
    );
    let mcp = process
        .as_mut()
        .ok_or_else(|| uncertain("exact-restore MCP process did not start"))?;
    let response = mcp
        .call(
            request_id,
            "tools/call",
            json!({"name": tool, "arguments": wrapper}),
        )
        .map_err(|message| uncertain(format!("exact-restore MCP call failed: {message}")))?;
    if response["id"].as_u64() != Some(request_id) || response.get("error").is_some() {
        return Err(uncertain(
            "MCP RPC response identity or envelope is invalid",
        ));
    }
    let content = response["result"]["content"]
        .as_array()
        .ok_or_else(|| uncertain("MCP tool response omitted content"))?;
    if content.len() != 1 {
        return Err(uncertain(
            "MCP exact-restore response must contain exactly one frame",
        ));
    }
    let text = content[0]["text"]
        .as_str()
        .ok_or_else(|| uncertain("MCP tool response omitted frame text"))?;
    if text.len() > MAX_FRAME_BYTES {
        return Err(uncertain(
            "exact-restore response wrapper exceeds the 16384-byte bound",
        ));
    }
    let response = super::super::super::gateway_json::parse(text.as_bytes())
        .map_err(|_| uncertain("Gateway response is invalid JSON or has duplicate keys"))?;
    // The canonical MCP exact-restore mapping projects the already validated
    // Gateway envelope to its neutral frame in `tools/call` content.  The
    // consumer therefore accepts only the pinned neutral frame here; a
    // Gateway wrapper or any other shape is a protocol violation.
    let frame = &response;
    let inner_bytes = canonical_bytes(frame).map_err(uncertain)?;
    if inner_bytes.len() > MAX_FRAME_BYTES
        || !valid_schema(neutral_validator().map_err(uncertain)?, frame)
        || response["contract"] != NEUTRAL_CONTRACT
        || response["schema_digest"] != NEUTRAL_SCHEMA_DIGEST
        || frame["correlation_id"] != request["message_id"]
        || frame["payload"]["operation_id"] != request["payload"]["operation_id"]
        || frame["payload"]["expected_owner"] != request["payload"]["expected_owner"]
        || frame["payload"]["request_digest"] != request_digest
    {
        return Err(uncertain(
            "Gateway response does not correlate to the exact request, owner, and operation",
        ));
    }
    let request_kind = request["kind"]
        .as_str()
        .ok_or_else(|| uncertain("exact-restore request kind is missing"))?;
    let response_kind = request_kind
        .strip_suffix("_request")
        .map(|kind| format!("{kind}_response"))
        .ok_or_else(|| uncertain("exact-restore request kind is invalid"))?;
    if frame["kind"] != "exact_restore_error_response" && frame["kind"] != response_kind {
        return Err(uncertain(
            "Gateway response phase differs from the exact-restore request",
        ));
    }
    Ok(frame.clone())
}

pub(super) fn ensure_profile(
    process: &mut Option<McpProcess>,
    config: &RuntimeConfig,
) -> Result<(), String> {
    let is_closed = process.as_mut().is_none_or(McpProcess::refresh_closed);
    if !is_closed {
        return Ok(());
    }
    if let Some(mut prior) = process.take() {
        prior.close()?;
    }
    let mut mcp = McpProcess::spawn_profile(config, "exact-restore-v1")?;
    if let Err(error) =
        super::super::super::runtime_v3_wire::initialize_mcp_profile(&mut mcp, "exact-restore-v1")
    {
        let close = mcp.close();
        return Err(match close {
            Ok(()) => error,
            Err(close_error) => format!("{error}; exact-restore MCP close failed: {close_error}"),
        });
    }
    *process = Some(mcp);
    Ok(())
}

pub(crate) fn request_frame(kind: &str, payload: Value) -> Result<Value, String> {
    let frame = json!({
        "contract": NEUTRAL_CONTRACT,
        "schema_digest": NEUTRAL_SCHEMA_DIGEST,
        "message_id": uuid::Uuid::new_v4().to_string(),
        "correlation_id": uuid::Uuid::new_v4().to_string(),
        "kind": kind,
        "payload": payload,
    });
    let bytes = canonical_bytes(&frame)?;
    if bytes.len() > MAX_FRAME_BYTES || !valid_schema(neutral_validator()?, &frame) {
        return Err(String::from(
            "exact-restore request violates the pinned schema or 16384-byte frame bound",
        ));
    }
    Ok(frame)
}
