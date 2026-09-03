// SPDX-License-Identifier: MIT

use super::{
    McpProcess, RuntimeConfig, observation_counts, play_card_observation_changed, require_kind,
    require_success, tool_call, trace_correlations, trace_lineage, wait_for_operation_settlement,
};
use serde_json::json;

pub(super) fn run(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
    let mut request_ids = vec![1_u64, 2_u64];
    require_success(
        &mcp.call(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "sts2-harness-runtime-v3-gameplay", "version": "0.0.0"}
            }),
        )?,
        "initialize",
    )?;
    let catalog = mcp.call(2, "tools/list", json!({}))?;
    require_success(&catalog, "tools/list")?;
    if catalog["result"]["revision"] != "runtime-v3-gameplay-mcp"
        || catalog["result"]["tools"]
            .as_array()
            .is_none_or(|tools| tools.len() != 3)
    {
        return Err(String::from(
            "MCP catalog did not advertise the exact Runtime-v3 gameplay catalog",
        ));
    }
    let context_session = config.mcp_session_id.as_str();
    request_ids.push(3);
    let before = tool_call(
        mcp,
        config,
        3,
        "get_state",
        json!({
            "instance_id": config.instance_id,
            "mcp_session_id": context_session,
            "lease_id": config.lease_id,
            "lease_epoch": config.lease_epoch,
            "generation": 0
        }),
    )?;
    require_kind(&before, "state_response")?;
    let before_generation = before["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v3 gameplay initial state omitted generation"))?;
    let before_observation = &before["observation"];
    let operation_id = "op-harness-runtime-v3-gameplay";
    let mut request_id = 4;
    request_ids.push(request_id);
    let submitted = tool_call(
        mcp,
        config,
        request_id,
        "submit_action",
        json!({
            "instance_id": config.instance_id,
            "mcp_session_id": context_session,
            "lease_id": config.lease_id,
            "lease_epoch": config.lease_epoch,
            "generation": before_generation,
            "operation_id": operation_id,
            "action_id": "play_card",
            "card_index": config.runtime_v3_card_index,
            "target_id": config.runtime_v3_target_id
        }),
    )?;
    request_id += 1;
    require_kind(&submitted, "action_response")?;
    let submitted_status = submitted["status"]
        .as_str()
        .ok_or_else(|| String::from("Runtime-v3 gameplay action omitted status"))?;
    if !matches!(submitted_status, "accepted" | "settled" | "unknown") {
        return Err(String::from("Runtime-v3 gameplay action was not admitted"));
    }
    let final_result = if submitted_status == "settled" {
        submitted.clone()
    } else {
        let reconcile_generation = submitted["generation"].as_u64().ok_or_else(|| {
            String::from("Runtime-v3 gameplay action omitted reconcile generation")
        })?;
        wait_for_operation_settlement(
            mcp,
            config,
            &mut request_id,
            &mut request_ids,
            operation_id,
            reconcile_generation,
        )?
    };
    if final_result["status"] != "settled"
        || final_result["effect_witness"]["kind"] != "play_card_settled"
        || final_result["observation"]["generation"] != final_result["generation"]
    {
        return Err(String::from(
            "Runtime-v3 gameplay reconciliation did not produce a fresh play_card witness",
        ));
    }
    let after_generation = final_result["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v3 gameplay settlement omitted generation"))?;
    if after_generation <= before_generation {
        return Err(String::from(
            "Runtime-v3 gameplay settlement did not advance generation",
        ));
    }
    request_ids.push(request_id);
    let after = tool_call(
        mcp,
        config,
        request_id,
        "get_state",
        json!({
            "instance_id": config.instance_id,
            "mcp_session_id": context_session,
            "lease_id": config.lease_id,
            "lease_epoch": config.lease_epoch,
            "generation": after_generation
        }),
    )?;
    require_kind(&after, "state_response")?;
    if after["generation"] != after_generation
        || after["observation"]["generation"] != after_generation
        || !play_card_observation_changed(before_observation, &after["observation"])
    {
        return Err(String::from(
            "Runtime-v3 gameplay post-state did not retain a card-play collection or energy change",
        ));
    }
    let correlation_ids = trace_correlations(&[
        ("initial_state", &before),
        ("submitted_action", &submitted),
        ("settled_action", &final_result),
        ("post_state", &after),
    ])?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": "runtime-v3-gameplay",
            "instance_id": config.instance_id,
            "session_id": config.session_id,
            "gateway_session_id": config.session_id,
            "mcp_session_id": config.mcp_session_id,
            "lineage": trace_lineage(config),
            "operation_id": operation_id,
            "request_ids": request_ids,
            "correlation_ids": correlation_ids,
            "before_generation": before_generation,
            "submitted_status": submitted_status,
            "after_generation": after_generation,
            "settlement_witness": {"kind": "play_card_settled", "generation": after_generation},
            "before_observation": observation_counts(before_observation),
            "after_observation": observation_counts(&after["observation"])
        }))
        .map_err(|error| format!("Runtime-v3 gameplay trace serialization failed: {error}"))?
    );
    Ok(())
}
