// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::ActionKind;

use super::mcp::McpProcess;
use super::mcp_process::McpProcessError;
use super::runtime_v3_settings::live_admission::admitted_live_episode;

#[path = "runtime_v3_wire_recovery.rs"]
mod recovery;
pub(super) use recovery::{initialize_recovery_mcp, recovery_call};
#[path = "runtime_v3_recovery_base64.rs"]
mod recovery_encoding;
pub(super) use recovery_encoding::decode as decode_recovery_action;

pub(super) const RUNTIME_V3_SCHEMA_DIGEST: &str =
    "0ae1d4d1525162da3059c028dcdb70df1d4d2dcf9620c5edd9b543e5f04aacc2";

const CATALOG_REVISION: &str = "runtime-v3-gameplay-mcp";
const EXPERT_CATALOG_REVISION: &str = "runtime-v4-expert-mcp";
const EXPERT_REST_ACTION_CATALOG_REVISION: &str = "runtime-v4-expert-rest-action-mcp";
const RECEIPT_QUERY_CATALOG_REVISION: &str = "coop-receipt-query-v1-mcp";
const SEEDED_RUN_CATALOG_REVISION: &str = "seeded-run-v1-mcp";
const EXACT_RESTORE_CATALOG_REVISION: &str = "exact-restore-v1-mcp";

include!("runtime_v3_wire_failure.rs");
include!("runtime_v3_wire_read_kind.rs");

#[path = "runtime_v3_wire_map_deadline.rs"]
mod map_deadline;
pub(super) use map_deadline::MapProfileDeadline;
#[path = "runtime_v3_wire_bounded_rpc.rs"]
mod bounded_rpc;
#[cfg(test)]
pub(super) use bounded_rpc::{
    has_catalog_reobserve, is_transient_gateway_rpc_error, is_transient_gateway_tool_error,
};
pub(super) use bounded_rpc::{
    rpc_call, rpc_call_catalog_read, rpc_call_recovery_read, rpc_call_with_deadline,
};

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

pub(super) fn catalog_reobserve(value: &Value) -> bool {
    value.as_object().is_some_and(|fields| fields.len() == 3)
        && value["correlation_id"].as_str().is_some()
        && value["recovery"] == "reobserve"
        && value["error_code"]
            .as_str()
            .is_some_and(catalog_recovery_code)
}

include!("runtime_v3_wire_refusal.rs");

include!("runtime_v3_wire_validation.rs");
#[path = "runtime_v3_lookup_catalog.rs"]
mod lookup_catalog;

fn validate_catalog(response: &Value, profile: &str) -> Result<(), String> {
    if profile == "negotiated-composition-v1" {
        return lookup_catalog::validate(response);
    }
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
        "exact-restore-v1" => (
            EXACT_RESTORE_CATALOG_REVISION,
            &[
                "sts2.exact_restore.begin",
                "sts2.exact_restore.put_chunk",
                "sts2.exact_restore.finish_blob",
                "sts2.exact_restore.commit",
                "sts2.exact_restore.lookup",
            ],
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
