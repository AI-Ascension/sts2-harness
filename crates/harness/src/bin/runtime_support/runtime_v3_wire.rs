// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::Digest;
use sts2_harness::ActionKind;

use super::mcp::McpProcess;

const CATALOG_REVISION: &str = "runtime-v3-gameplay-mcp";
pub(super) const RUNTIME_V3_SCHEMA_DIGEST: &str =
    "8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63";

pub(super) fn initialize_mcp(mcp: &mut McpProcess) -> Result<(), String> {
    let initialize = rpc_call(
        mcp,
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "sts2-harness-runtime-v3", "version": "0.0.0"}
        }),
    )?;
    if initialize.get("result").is_none() {
        return Err(String::from("MCP initialize omitted result"));
    }
    let catalog = rpc_call(mcp, 2, "tools/list", json!({}))?;
    validate_catalog(&catalog)
}

pub(super) fn initialize_recovery_mcp(mcp: &mut McpProcess) -> Result<(), String> {
    let initialize = rpc_call(
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
    let catalog = rpc_call(mcp, 2, "tools/list", json!({}))?;
    validate_recovery_catalog(&catalog)
}

pub(super) fn recovery_call(
    mcp: &mut McpProcess,
    id: u64,
    mcp_session_id: &str,
    name: &str,
    expected_kind: &str,
    payload: Value,
) -> Result<Value, String> {
    let response = rpc_call(
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

pub(super) fn rpc_call(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let timeout = request_timeout(method, &params)?;
    let catalog_read = method == "tools/call" && params["name"] == "sts2.legal_actions";
    let response = mcp.call_with_timeout(id, method, params, timeout)?;
    if response.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(format!(
            "MCP {method} response identity does not match request"
        ));
    }
    if response.get("error").is_some() {
        if std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true") {
            // Preserve the numeric RPC category without the remote message or data payload.
            eprintln!(
                "MCP RPC failure: code={:?}",
                response["error"]["code"].as_i64()
            );
        }
        return Err(format!("MCP {method} returned an RPC error"));
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
                || has_recovery_envelope(&response)
                || (catalog_read && has_catalog_reobserve(&response, id)))
        {
            return Err(format!("MCP {method} returned a tool error"));
        }
    }
    Ok(response)
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

fn has_gameplay_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value["protocol_version"] == "runtime-v3-gameplay")
}

fn has_recovery_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| {
            value["contract"] == "watchdog-recovery-v1"
                && value["schema_digest"].as_str() == Some(sts2_harness::RECOVERY_SCHEMA_DIGEST)
        })
}

fn request_timeout(method: &str, params: &Value) -> Result<std::time::Duration, String> {
    let wait = if method == "tools/call" && params["name"] == "sts2.wait_for_transition" {
        params["arguments"]["wait_for_millis"]
            .as_u64()
            .filter(|value| *value <= 120_000)
            .ok_or_else(|| String::from("MCP transition wait is outside its bound"))?
    } else {
        0
    };
    Ok(std::time::Duration::from_millis(wait + 5_000))
}

fn validate_catalog(response: &Value) -> Result<(), String> {
    let result = response
        .get("result")
        .ok_or_else(|| String::from("MCP tools/list omitted result"))?;
    if result.get("revision").and_then(Value::as_str) != Some(CATALOG_REVISION) {
        return Err(String::from("MCP catalog is not runtime-v3-gameplay-mcp"));
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("MCP catalog omitted tools"))?;
    let expected = [
        "sts2.observe",
        "sts2.legal_actions",
        "sts2.dispatch_action",
        "sts2.wait_for_transition",
        "sts2.reobserve",
        "sts2.recover",
    ];
    if tools.len() != expected.len()
        || tools
            .iter()
            .zip(expected)
            .any(|(tool, expected)| tool.get("name").and_then(Value::as_str) != Some(expected))
    {
        return Err(String::from(
            "MCP catalog does not expose the exact six-tool surface",
        ));
    }
    Ok(())
}

fn validate_recovery_catalog(response: &Value) -> Result<(), String> {
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

pub(super) const fn action_kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::StartRun => "start_run",
        ActionKind::SelectMapNode => "select_map_node",
        ActionKind::PlayCard => "play_card",
        ActionKind::EndTurn => "end_turn",
        ActionKind::ChooseReward => "choose_reward",
        ActionKind::SkipReward => "skip_reward",
        ActionKind::Proceed => "proceed",
        ActionKind::ConfirmSelection => "confirm_selection",
        ActionKind::CancelSelection => "cancel_selection",
        ActionKind::ShopPurchase => "shop_purchase",
        ActionKind::ShopRemove => "shop_remove",
        ActionKind::Rest => "rest",
        ActionKind::Smith => "smith",
        ActionKind::EventChoice => "event_choice",
        ActionKind::SelectCard => "select_card",
        ActionKind::ConfirmVictory => "confirm_victory",
        ActionKind::SaveQuit => "save_quit",
    }
}

/// The canonical recovery action is the complete legal-action envelope, not merely the inner
/// payload sent to the frozen gameplay tool. Its bytes are retained before dispatch and are the
/// only bytes accepted for historical recovery.
pub(super) fn canonical_action_bytes(action_id: &str, payload: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"action": payload, "action_id": action_id}))
        .map_err(|error| format!("cannot encode canonical runtime-v3 action: {error}"))
}

pub(super) fn canonical_action_digest(action_id: &str, payload: &Value) -> Result<String, String> {
    let bytes = canonical_action_bytes(action_id, payload)?;
    Ok(format!("{:x}", sha2::Sha256::digest(bytes)))
}

pub(super) const fn stage_name(stage: sts2_harness::EpisodeStage) -> &'static str {
    match stage {
        sts2_harness::EpisodeStage::Setup => "setup",
        sts2_harness::EpisodeStage::Map => "map",
        sts2_harness::EpisodeStage::Combat => "combat",
        sts2_harness::EpisodeStage::Reward => "reward",
        sts2_harness::EpisodeStage::Shop => "shop",
        sts2_harness::EpisodeStage::Event => "event",
        sts2_harness::EpisodeStage::Rest => "rest",
        sts2_harness::EpisodeStage::Selection => "selection",
        sts2_harness::EpisodeStage::Victory => "victory",
        sts2_harness::EpisodeStage::Defeat => "defeat",
        sts2_harness::EpisodeStage::Recovery => "recovery",
        sts2_harness::EpisodeStage::Unknown => "unknown",
    }
}

pub(super) fn port_error(
    code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> sts2_harness::PortError {
    sts2_harness::PortError::new(code, message, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_recovery_requires_exact_shape_and_correlation() {
        for code in [
            "stale_generation",
            "host_not_configured",
            "host_observation_unavailable",
        ] {
            let mut body =
                json!({"correlation_id":"42", "error_code":code, "recovery":"reobserve"});
            let response = |body: &Value| json!({"result":{"isError":true,"content":[{"text":body.to_string()}]}});
            assert!(has_catalog_reobserve(&response(&body), 42));
            assert!(!has_catalog_reobserve(&response(&body), 43));
            body["private"] = json!("extra");
            assert!(!has_catalog_reobserve(&response(&body), 42));
        }
        for code in ["unauthorized", "timeout", "unknown"] {
            assert!(!catalog_reobserve(
                &json!({"correlation_id":"42","error_code":code,"recovery":"reobserve"})
            ));
        }
    }
    #[test]
    fn gameplay_unknown_remains_available_for_receipt_validation() {
        let envelope = json!({"protocol_version":"runtime-v3-gameplay", "status":"unknown",
            "error_code":"settlement_unproven"});
        let mut response = json!({"result":{"isError":true,
            "content":[{"text":envelope.to_string()}]}});
        assert!(has_gameplay_envelope(&response));
        response["result"]["content"][0]["text"] = json!("gateway error -32005: rejected");
        assert!(!has_gameplay_envelope(&response));
        response["result"]["content"][0]["text"] = json!("{}");
        assert!(!has_gameplay_envelope(&response));
    }
    #[test]
    fn transition_wait_budget_includes_requested_semantic_wait() -> Result<(), String> {
        let mut params =
            json!({"name":"sts2.wait_for_transition","arguments":{"wait_for_millis":120_000}});
        assert_eq!(request_timeout("tools/call", &params)?.as_millis(), 125_000);
        assert_eq!(request_timeout("initialize", &params)?.as_millis(), 5_000);
        params["arguments"]["wait_for_millis"] = json!(120_001);
        assert!(request_timeout("tools/call", &params).is_err());
        params["arguments"]["wait_for_millis"] = Value::Null;
        assert!(request_timeout("tools/call", &params).is_err());
        Ok(())
    }
}
