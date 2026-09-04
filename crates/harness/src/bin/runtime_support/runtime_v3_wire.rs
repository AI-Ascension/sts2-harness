// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::ActionKind;

use super::mcp::McpProcess;

const CATALOG_REVISION: &str = "runtime-v3-gameplay-mcp";

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

pub(super) fn rpc_call(
    mcp: &mut McpProcess,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let timeout = request_timeout(method, &params)?;
    let response = mcp.call_with_timeout(id, method, params, timeout)?;
    if response.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(format!(
            "MCP {method} response identity does not match request"
        ));
    }
    if response.get("error").is_some() {
        return Err(format!("MCP {method} returned an RPC error"));
    }
    if response
        .get("result")
        .and_then(|result| result.get("isError"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(format!("MCP {method} returned a tool error"));
    }
    Ok(response)
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
