// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::ActionKind;

use super::mcp::McpProcess;
use super::mcp_process::McpProcessError;

#[path = "runtime_v3_wire_recovery.rs"]
mod recovery;
pub(super) use recovery::{initialize_recovery_mcp, recovery_call};
#[path = "runtime_v3_recovery_base64.rs"]
mod recovery_encoding;
pub(super) use recovery_encoding::decode as decode_recovery_action;

pub(super) const RUNTIME_V3_SCHEMA_DIGEST: &str =
    "8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63";

const CATALOG_REVISION: &str = "runtime-v3-gameplay-mcp";
const EXPERT_CATALOG_REVISION: &str = "runtime-v4-expert-mcp";
const EXPERT_REST_ACTION_CATALOG_REVISION: &str = "runtime-v4-expert-rest-action-mcp";
const RECEIPT_QUERY_CATALOG_REVISION: &str = "coop-receipt-query-v1-mcp";
const SEEDED_RUN_CATALOG_REVISION: &str = "seeded-run-v1-mcp";

include!("runtime_v3_wire_failure.rs");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RpcReadKind {
    None,
    Catalog,
    Recovery,
}

pub(super) fn initialize_mcp_profile(mcp: &mut McpProcess, profile: &str) -> Result<(), String> {
    initialize_mcp_profile_classified(mcp, profile).map_err(|error| error.to_string())
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn initialize_mcp(mcp: &mut McpProcess) -> Result<(), String> {
    initialize_mcp_profile(mcp, "runtime-v3-gameplay")
}

pub(super) fn initialize_mcp_profile_classified(
    mcp: &mut McpProcess,
    profile: &str,
) -> Result<(), RpcFailure> {
    let initialize = rpc_call_recovery_read(
        mcp,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "sts2-harness-runtime", "version": profile}
        }),
    )?;
    if initialize.get("result").is_none() {
        return Err(RpcFailure::terminal("MCP initialize omitted result"));
    }
    let catalog = rpc_call_recovery_read(mcp, 2, "tools/list", json!({}))?;
    validate_catalog(&catalog, profile).map_err(RpcFailure::terminal)
}

pub(super) fn rpc_call(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    let catalog_read = method == "tools/call" && params["name"] == "sts2.legal_actions";
    rpc_call_with_read_kind(
        mcp,
        id,
        method,
        params,
        if catalog_read {
            RpcReadKind::Catalog
        } else {
            RpcReadKind::None
        },
    )
}

pub(super) fn rpc_call_catalog_read(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    rpc_call_with_read_kind(mcp, id, method, params, RpcReadKind::Catalog)
}

pub(super) fn rpc_call_recovery_read(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, RpcFailure> {
    rpc_call_with_read_kind(mcp, id, method, params, RpcReadKind::Recovery)
}

fn rpc_call_with_read_kind(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
    read_kind: RpcReadKind,
) -> Result<Value, RpcFailure> {
    let timeout = request_timeout(method, &params).map_err(RpcFailure::terminal)?;
    let response = mcp
        .call_with_timeout_classified(id, method, params, timeout)
        .map_err(RpcFailure::from_mcp)?;
    if response.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(RpcFailure::terminal(format!(
            "MCP {method} response identity does not match request"
        )));
    }
    if response.get("error").is_some() {
        if std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true") {
            // Preserve the numeric RPC category without the remote message or data payload.
            eprintln!(
                "MCP RPC failure: code={:?}",
                response["error"]["code"].as_i64()
            );
        }
        if read_kind != RpcReadKind::None && is_transient_gateway_rpc_error(&response) {
            return Err(RpcFailure::transient(
                "MCP recovery read was temporarily unavailable",
            ));
        }
        return Err(RpcFailure::terminal(format!(
            "MCP {method} returned an RPC error"
        )));
    }
    if response
        .get("result")
        .and_then(|result| result.get("isError"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        // MCP marks unknown/rejected gameplay receipts as tool errors. Preserve their
        // envelope for the caller's full identity/schema validation and reconciliation.
        if method != "tools/call"
            || !(has_gameplay_envelope(&response)
                || has_expert_action_envelope(&response)
                || has_expert_rest_action_envelope(&response)
                || has_receipt_query_envelope(&response)
                || (read_kind == RpcReadKind::Recovery && has_recovery_envelope(&response))
                || (read_kind == RpcReadKind::Catalog && has_catalog_reobserve(&response, id)))
        {
            if read_kind != RpcReadKind::None && is_transient_gateway_tool_error(&response) {
                return Err(RpcFailure::transient(
                    "MCP recovery read was temporarily unavailable",
                ));
            }
            return Err(RpcFailure::terminal(format!(
                "MCP {method} returned a tool error"
            )));
        }
    }
    Ok(response)
}

fn is_transient_gateway_rpc_error(response: &Value) -> bool {
    matches!(response["error"]["code"].as_i64(), Some(-32003 | -32008))
}

fn is_transient_gateway_tool_error(response: &Value) -> bool {
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

fn has_catalog_reobserve(response: &Value, id: u64) -> bool {
    let correlation = id.to_string();
    response["result"]["content"][0]["text"]
        .as_str()
        .filter(|text| text.len() <= 1024)
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| {
            catalog_reobserve(&value)
                && value["correlation_id"].as_str() == Some(correlation.as_str())
        })
}

pub(super) fn catalog_reobserve(value: &Value) -> bool {
    value.as_object().is_some_and(|fields| fields.len() == 3)
        && value["correlation_id"].as_str().is_some()
        && value["recovery"] == "reobserve"
        && matches!(
            value["error_code"].as_str(),
            Some("stale_generation" | "host_not_configured" | "host_observation_unavailable")
        )
}

include!("runtime_v3_wire_validation.rs");

fn validate_catalog(response: &Value, profile: &str) -> Result<(), String> {
    let result = response
        .get("result")
        .ok_or_else(|| String::from("MCP tools/list omitted result"))?;
    let (revision, expected): (&str, &[&str]) = match profile {
        "runtime-v3-gameplay" => (
            CATALOG_REVISION,
            &[
                "sts2.observe",
                "sts2.legal_actions",
                "sts2.dispatch_action",
                "sts2.wait_for_transition",
                "sts2.reobserve",
                "sts2.recover",
            ],
        ),
        "runtime-v4-expert" => (
            EXPERT_CATALOG_REVISION,
            &[
                "sts2.expert_state",
                "sts2.expert_action",
                "sts2.expert_reconcile",
            ],
        ),
        "runtime-v4-expert-rest-action" => (
            EXPERT_REST_ACTION_CATALOG_REVISION,
            &[
                "sts2.expert_state",
                "sts2.expert_rest_action",
                "sts2.expert_rest_reconcile",
            ],
        ),
        "coop-receipt-query-v1" => (RECEIPT_QUERY_CATALOG_REVISION, &["sts2.coop_receipt_query"]),
        "seeded-run-v1" => (
            SEEDED_RUN_CATALOG_REVISION,
            &["start_seeded_run", "reconcile_seeded_run"],
        ),
        _ => return Err(String::from("MCP profile is unsupported")),
    };
    if result.get("revision").and_then(Value::as_str) != Some(revision) {
        return Err(String::from("MCP catalog is not runtime-v3-gameplay-mcp"));
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("MCP catalog omitted tools"))?;
    if tools.len() != expected.len()
        || tools
            .iter()
            .zip(expected)
            .any(|(tool, expected)| tool.get("name").and_then(Value::as_str) != Some(expected))
    {
        return Err(String::from(
            "MCP catalog does not expose the exact profile tool surface",
        ));
    }
    Ok(())
}

pub(super) fn combine_cleanup(
    error: String,
    close: Result<(), String>,
    release: Result<(), String>,
) -> String {
    let mut message = error;
    if let Err(close_error) = close {
        message.push_str(&format!("; MCP cleanup failed: {close_error}"));
    }
    if let Err(release_error) = release {
        message.push_str(&format!("; lease release failed: {release_error}"));
    }
    message
}

include!("runtime_v3_wire_action_kind.rs");

/// The canonical recovery action is the complete legal-action envelope, not merely the inner
/// payload sent to the frozen gameplay tool. Its bytes are retained before dispatch and are the
/// only bytes accepted for historical recovery.
pub(super) fn canonical_action_bytes(action_id: &str, payload: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"action": payload, "action_id": action_id}))
        .map_err(|error| format!("cannot encode canonical runtime-v3 action: {error}"))
}

pub(super) fn canonical_action_digest(action_id: &str, payload: &Value) -> Result<String, String> {
    let bytes = canonical_action_bytes(action_id, payload)?;
    Ok(sts2_harness::sha256_hex(bytes))
}

include!("runtime_v3_wire_stage.rs");

pub(super) fn port_error(
    code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> sts2_harness::PortError {
    sts2_harness::PortError::new(code, message, retryable)
}

#[cfg(test)]
mod tests {
    include!("runtime_v3_wire_tests.rs");
}
