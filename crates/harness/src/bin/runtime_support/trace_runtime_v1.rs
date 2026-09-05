// SPDX-License-Identifier: MIT

use super::{McpProcess, RuntimeConfig, require_success, tool_call};
use serde_json::json;

pub(super) fn run(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
    require_success(
        &mcp.call(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "sts2-harness-runtime", "version": "0.0.0"}
            }),
        )?,
        "initialize",
    )?;
    let catalog = mcp.call(2, "tools/list", json!({}))?;
    require_success(&catalog, "tools/list")?;
    if catalog["result"]["revision"] != "runtime-v1-mcp" {
        return Err(String::from("MCP catalog did not advertise runtime-v1"));
    }

    let before = tool_call(
        mcp,
        config,
        3,
        "get_state",
        json!({"instance_id": config.instance_id, "mcp_session_id": config.mcp_session_id}),
    )?;
    let generation = before["generation"]
        .as_u64()
        .ok_or_else(|| String::from("initial state omitted generation"))?;
    let accepted = tool_call(
        mcp,
        config,
        4,
        "submit_action",
        json!({
            "instance_id": config.instance_id,
            "mcp_session_id": config.mcp_session_id,
            "generation": generation,
            "action_id": "show_runtime_probe"
        }),
    )?;
    if accepted["status"] != "accepted"
        || accepted["effect_witness"]["kind"] != "status_overlay_visible"
        || accepted["observation"]["overlay_visible"] != true
        || accepted["generation"].as_u64() != Some(generation + 1)
    {
        return Err(String::from(
            "accepted action did not produce a fresh visible witness",
        ));
    }
    let stale = tool_call(
        mcp,
        config,
        5,
        "submit_action",
        json!({
            "instance_id": config.instance_id,
            "mcp_session_id": config.mcp_session_id,
            "generation": generation,
            "action_id": "show_runtime_probe"
        }),
    )?;
    if stale["status"] != "rejected" || stale["error_code"] != "sts2.game-mod/stale_generation" {
        return Err(String::from(
            "stale generation was not rejected with a stable identity",
        ));
    }
    let after = tool_call(
        mcp,
        config,
        6,
        "get_state",
        json!({"instance_id": config.instance_id, "mcp_session_id": config.mcp_session_id}),
    )?;
    if after["generation"].as_u64() != Some(generation + 1)
        || after["observation"]["overlay_visible"] != true
        || after["observation"]["action_count"].as_u64() != Some(1)
    {
        return Err(String::from(
            "fresh post-action state did not retain the witnessed effect",
        ));
    }
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": "runtime-v1",
            "instance_id": config.instance_id,
            "session_id": config.session_id,
            "before_generation": generation,
            "after_generation": after["generation"],
            "accepted_effect": {"kind": "status_overlay_visible", "generation": generation + 1},
            "stale_rejection": stale["error_code"],
            "observation": {
                "overlay_visible": true,
                "action_count": 1
            }
        }))
        .map_err(|error| format!("trace serialization failed: {error}"))?
    );
    Ok(())
}
