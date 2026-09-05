// SPDX-License-Identifier: MIT

use super::{
    McpProcess, RuntimeConfig, require_kind, require_success, tool_call, trace_correlations,
    trace_lineage, wait_for_operation_settlement, wait_for_v2_player_turn,
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
                "clientInfo": {"name": "sts2-harness-runtime-v2", "version": "0.0.0"}
            }),
        )?,
        "initialize",
    )?;
    let catalog = mcp.call(2, "tools/list", json!({}))?;
    require_success(&catalog, "tools/list")?;
    if catalog["result"]["revision"] != "runtime-v2-mcp"
        || catalog["result"]["tools"]
            .as_array()
            .is_none_or(|tools| tools.len() != 3)
    {
        return Err(String::from(
            "MCP catalog did not advertise the exact Runtime-v2 catalog",
        ));
    }

    let context_session = config.mcp_session_id.as_str();
    let mut request_id = 3;
    let before = wait_for_v2_player_turn(mcp, config, &mut request_id, &mut request_ids)?;
    require_kind(&before, "state_response")?;
    let before_generation = before["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v2 initial state omitted generation"))?;
    let operation_id = "op-harness-runtime-v2";
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
            "action_id": "end_turn"
        }),
    )?;
    request_id += 1;
    require_kind(&submitted, "action_response")?;
    let submitted_status = submitted["status"]
        .as_str()
        .ok_or_else(|| String::from("Runtime-v2 action omitted status"))?;
    if !matches!(submitted_status, "accepted" | "settled" | "unknown") {
        return Err(String::from(
            "Runtime-v2 action returned unsupported status",
        ));
    }
    let operation_generation = submitted["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v2 action omitted generation"))?;
    let reconciled = wait_for_operation_settlement(
        mcp,
        config,
        &mut request_id,
        &mut request_ids,
        operation_id,
        operation_generation,
    )?;
    if reconciled["status"] != "settled"
        || reconciled["effect_witness"]["kind"] != "turn_end_settled"
        || reconciled["observation"]["generation"] != reconciled["generation"]
        || reconciled["generation"].as_u64() != Some(before_generation + 1)
    {
        return Err(String::from(
            "Runtime-v2 reconciliation did not produce a fresh settled witness",
        ));
    }
    let after_generation = reconciled["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v2 reconciliation omitted generation"))?;

    request_ids.push(request_id);
    let stale = tool_call(
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
            "operation_id": "op-harness-runtime-v2-stale",
            "action_id": "end_turn"
        }),
    )?;
    request_id += 1;
    if stale["status"] != "rejected" || stale["error_code"] != "sts2.game-core/stale_generation" {
        return Err(String::from(
            "Runtime-v2 stale generation was not rejected before a second mutation",
        ));
    }
    request_ids.push(request_id);
    let duplicate_replay = tool_call(
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
            "action_id": "end_turn"
        }),
    )?;
    request_id += 1;
    if duplicate_replay["status"] != "settled"
        || duplicate_replay["effect_witness"]["kind"] != "turn_end_settled"
        || duplicate_replay["generation"].as_u64() != Some(after_generation)
    {
        return Err(String::from(
            "Runtime-v2 exact duplicate did not replay its settled result",
        ));
    }
    request_ids.push(request_id);
    let duplicate_conflict = tool_call(
        mcp,
        config,
        request_id,
        "submit_action",
        json!({
            "instance_id": config.instance_id,
            "mcp_session_id": context_session,
            "lease_id": config.lease_id,
            "lease_epoch": config.lease_epoch,
            "generation": after_generation,
            "operation_id": operation_id,
            "action_id": "end_turn"
        }),
    )?;
    request_id += 1;
    if duplicate_conflict["status"] != "rejected"
        || duplicate_conflict["error_code"] != "idempotency_conflict"
    {
        return Err(String::from(
            "Runtime-v2 conflicting operation reuse was not rejected without re-dispatch",
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
    if after["generation"].as_u64() != Some(after_generation)
        || after["observation"]["generation"].as_u64() != Some(after_generation)
    {
        return Err(String::from(
            "Runtime-v2 post-action state did not retain the settled generation",
        ));
    }
    let correlation_ids = trace_correlations(&[
        ("initial_state", &before),
        ("submitted_action", &submitted),
        ("settled_action", &reconciled),
        ("stale_action", &stale),
        ("duplicate_replay", &duplicate_replay),
        ("duplicate_conflict", &duplicate_conflict),
        ("post_state", &after),
    ])?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": "runtime-v2",
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
            "reconciled_status": reconciled["status"],
            "after_generation": after_generation,
            "stale_rejection": stale["error_code"],
            "duplicate_replay_status": duplicate_replay["status"],
            "duplicate_conflict": duplicate_conflict["error_code"],
            "settlement_witness": {"kind": "turn_end_settled", "generation": after_generation}
        }))
        .map_err(|error| format!("Runtime-v2 trace serialization failed: {error}"))?
    );
    Ok(())
}
