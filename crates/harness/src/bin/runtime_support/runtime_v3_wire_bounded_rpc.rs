// SPDX-License-Identifier: MIT

use super::{MapProfileDeadline, McpProcess, RpcFailure, RpcReadKind, Value};

pub(in super::super) fn rpc_call(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    let read_kind = super::read_kind_for_dispatch(method, &params);
    rpc_call_with_read_kind(mcp, id, method, params, read_kind, None)
}

pub(in super::super) fn rpc_call_catalog_read(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    rpc_call_with_read_kind(mcp, id, method, params, RpcReadKind::Catalog, None)
}

pub(in super::super) fn rpc_call_recovery_read(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    rpc_call_with_read_kind(mcp, id, method, params, RpcReadKind::Recovery, None)
}

pub(in super::super) fn rpc_call_with_deadline(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
    deadline: &MapProfileDeadline,
) -> Result<Value, RpcFailure> {
    let read_kind = super::read_kind_for_dispatch(method, &params);
    rpc_call_with_read_kind(mcp, id, method, params, read_kind, Some(deadline))
}

fn rpc_call_with_read_kind(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
    read_kind: RpcReadKind,
    deadline: Option<&MapProfileDeadline>,
) -> Result<Value, RpcFailure> {
    let request_timeout = super::request_timeout(method, &params).map_err(RpcFailure::terminal)?;
    let response = match deadline {
        Some(deadline) => deadline
            .with_rpc_timeout(request_timeout, |timeout| {
                mcp.call_with_timeout_classified(id, method, params, timeout)
            })
            .ok_or_else(|| RpcFailure::terminal("MCP map profile deadline expired"))?,
        None => mcp.call_with_timeout_classified(id, method, params, request_timeout),
    }
    .map_err(RpcFailure::from_mcp)?;
    if response.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(RpcFailure::terminal(format!(
            "MCP {method} response identity does not match request"
        )));
    }
    if response.get("error").is_some() {
        if super::admitted_live_episode() {
            eprintln!(
                "MCP RPC failure: code={:?}",
                response["error"]["code"].as_i64()
            );
        }
        if super::classifies_transient_gateway_faults(read_kind)
            && is_transient_gateway_rpc_error(&response)
        {
            return Err(RpcFailure::transient(
                "MCP recovery read was temporarily unavailable",
            ));
        }
        return Err(RpcFailure::terminal(format!(
            "MCP {method} returned an RPC error"
        )));
    }
    if response["result"]["isError"].as_bool() == Some(true)
        && !accepts_tool_error(&response, method, id, read_kind)
    {
        if super::classifies_transient_gateway_faults(read_kind)
            && is_transient_gateway_tool_error(&response)
        {
            return Err(RpcFailure::transient(
                "MCP recovery read was temporarily unavailable",
            ));
        }
        return Err(RpcFailure::terminal(format!(
            "MCP {method} returned a tool error"
        )));
    }
    Ok(response)
}

fn accepts_tool_error(response: &Value, method: &str, id: u64, read_kind: RpcReadKind) -> bool {
    method == "tools/call"
        && (super::has_gameplay_envelope(response)
            || super::has_expert_action_envelope(response)
            || super::has_expert_rest_action_envelope(response)
            || super::has_receipt_query_envelope(response)
            || super::has_bootstrap_error_envelope(response)
            || (read_kind == RpcReadKind::Recovery && super::has_recovery_envelope(response))
            || (read_kind == RpcReadKind::Catalog && has_catalog_reobserve(response, id)))
}

pub(in super::super) fn is_transient_gateway_rpc_error(response: &Value) -> bool {
    matches!(response["error"]["code"].as_i64(), Some(-32003 | -32008))
}

pub(in super::super) fn is_transient_gateway_tool_error(response: &Value) -> bool {
    response["result"]["content"]
        .as_array()
        .is_some_and(|content| content.len() == 1)
        && matches!(
            response["result"]["content"][0]["text"].as_str(),
            Some(
                "gateway error -32003: gateway is unavailable"
                    | "gateway error -32008: gateway request timed out"
            )
        )
}

pub(in super::super) fn has_catalog_reobserve(response: &Value, id: u64) -> bool {
    let correlation = id.to_string();
    response["result"]["content"][0]["text"]
        .as_str()
        .filter(|text| text.len() <= 1024)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| {
            super::catalog_reobserve(&value)
                && value["correlation_id"].as_str() == Some(correlation.as_str())
        })
}
