// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::config::RuntimeConfig;
use super::http::GatewayClient;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    let client = GatewayClient::new(&config)?;
    let allocation = client.request(
        "POST",
        "/v1/sessions/allocate",
        &json!({
            "instance_id": config.instance_id,
            "caller_id": config.caller_id,
            "session_id": config.session_id
        }),
        BTreeMap::from([(
            String::from("x-mcp-session-id"),
            config.mcp_session_id.clone(),
        )]),
    )?;
    validate_allocation(&allocation, &config)?;
    let mut mcp = match McpProcess::spawn(&config) {
        Ok(process) => process,
        Err(error) => {
            let release = client.request(
                "POST",
                &format!("/v1/instances/{}/release", config.instance_id),
                &Value::Null,
                identity_headers(&config, "release-0001"),
            );
            return match release {
                Ok(_) => Err(error),
                Err(release_error) => Err(format!(
                    "{error}; allocated lease release also failed: {release_error}"
                )),
            };
        }
    };
    let trace_result = run_trace(&mut mcp, &config);
    let close_result = mcp.close();
    let release_result = client.request(
        "POST",
        &format!("/v1/instances/{}/release", config.instance_id),
        &Value::Null,
        identity_headers(&config, "release-0001"),
    );
    trace_result?;
    close_result?;
    release_result.map(|_| ())?;
    Ok(())
}

fn run_trace(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
    if config.runtime_profile == "runtime-v2" {
        return run_trace_v2(mcp, config);
    }
    if config.runtime_profile == "runtime-v3-gameplay" {
        return run_trace_v3_gameplay(mcp, config);
    }
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
        3,
        "get_state",
        json!({"instance_id": config.instance_id, "mcp_session_id": config.mcp_session_id}),
    )?;
    let generation = before["generation"]
        .as_u64()
        .ok_or_else(|| String::from("initial state omitted generation"))?;
    let accepted = tool_call(
        mcp,
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
            "accepted_effect": accepted["effect_witness"],
            "stale_rejection": stale["error_code"],
            "observation": after["observation"]
        }))
        .map_err(|error| format!("trace serialization failed: {error}"))?
    );
    Ok(())
}

fn wait_for_v2_player_turn(
    mcp: &mut McpProcess,
    config: &RuntimeConfig,
    request_id: &mut u64,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(config.wait_for_combat_seconds);
    loop {
        let observation = tool_call(
            mcp,
            *request_id,
            "get_state",
            json!({
                "instance_id": config.instance_id,
                "mcp_session_id": config.mcp_session_id,
                "lease_id": config.lease_id,
                "lease_epoch": config.lease_epoch,
                "generation": 0
            }),
        )?;
        *request_id += 1;
        if observation["observation"]["combat_phase"] == "combat/player_turn" {
            return Ok(observation);
        }
        if config.wait_for_combat_seconds == 0 || Instant::now() >= deadline {
            return Err(format!(
                "Runtime-v2 host did not reach combat/player_turn; observed phase {}",
                observation["observation"]["combat_phase"]
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn wait_for_operation_settlement(
    mcp: &mut McpProcess,
    config: &RuntimeConfig,
    request_id: &mut u64,
    operation_id: &str,
    generation: u64,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(config.settlement_timeout_seconds);
    loop {
        let reconciled = tool_call(
            mcp,
            *request_id,
            "reconcile_action",
            json!({
                "instance_id": config.instance_id,
                "mcp_session_id": config.mcp_session_id,
                "lease_id": config.lease_id,
                "lease_epoch": config.lease_epoch,
                "generation": generation,
                "operation_id": operation_id
            }),
        )?;
        *request_id += 1;
        require_kind(&reconciled, "reconcile_response")?;
        if reconciled["status"] == "settled" {
            return Ok(reconciled);
        }
        if config.settlement_timeout_seconds == 0 || Instant::now() >= deadline {
            return Err(format!(
                "Runtime operation did not settle before the bounded timeout: {reconciled}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn run_trace_v3_gameplay(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
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
    let before = tool_call(
        mcp,
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
    let submitted = tool_call(
        mcp,
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
        return Err(format!(
            "Runtime-v3 gameplay action was not admitted: {submitted}"
        ));
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
    let after = tool_call(
        mcp,
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
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": "runtime-v3-gameplay",
            "instance_id": config.instance_id,
            "session_id": config.session_id,
            "mcp_session_id": config.mcp_session_id,
            "before_generation": before_generation,
            "submitted_status": submitted_status,
            "after_generation": after_generation,
            "settlement_witness": final_result["effect_witness"],
            "before_observation": before_observation,
            "after_observation": after["observation"]
        }))
        .map_err(|error| format!("Runtime-v3 gameplay trace serialization failed: {error}"))?
    );
    Ok(())
}

fn play_card_observation_changed(before: &Value, after: &Value) -> bool {
    before["hand_count"] != after["hand_count"]
        || before["energy"] != after["energy"]
        || before["draw_pile_count"] != after["draw_pile_count"]
        || before["discard_pile_count"] != after["discard_pile_count"]
        || before["exhaust_pile_count"] != after["exhaust_pile_count"]
}

fn run_trace_v2(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
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
    let before = wait_for_v2_player_turn(mcp, config, &mut request_id)?;
    require_kind(&before, "state_response")?;
    let before_generation = before["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v2 initial state omitted generation"))?;
    let operation_id = "op-harness-runtime-v2";
    let submitted = tool_call(
        mcp,
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
        return Err(format!(
            "Runtime-v2 action returned unsupported status {submitted_status}"
        ));
    }
    let operation_generation = submitted["generation"]
        .as_u64()
        .ok_or_else(|| String::from("Runtime-v2 action omitted generation"))?;
    let reconciled = wait_for_operation_settlement(
        mcp,
        config,
        &mut request_id,
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

    let stale = tool_call(
        mcp,
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
    let duplicate_replay = tool_call(
        mcp,
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
    let duplicate_conflict = tool_call(
        mcp,
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
    let after = tool_call(
        mcp,
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
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": "runtime-v2",
            "instance_id": config.instance_id,
            "session_id": config.session_id,
            "mcp_session_id": config.mcp_session_id,
            "before_generation": before_generation,
            "submitted_status": submitted_status,
            "reconciled_status": reconciled["status"],
            "after_generation": after_generation,
            "stale_rejection": stale["error_code"],
            "duplicate_replay_status": duplicate_replay["status"],
            "duplicate_conflict": duplicate_conflict["error_code"],
            "settlement_witness": reconciled["effect_witness"]
        }))
        .map_err(|error| format!("Runtime-v2 trace serialization failed: {error}"))?
    );
    Ok(())
}

fn require_kind(value: &Value, expected: &str) -> Result<(), String> {
    if value["kind"] == expected {
        Ok(())
    } else {
        Err(format!("Runtime-v2 response kind was not {expected}"))
    }
}

fn tool_call(mcp: &mut McpProcess, id: u64, name: &str, arguments: Value) -> Result<Value, String> {
    let response = mcp.call(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )?;
    require_success(&response, name)?;
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| String::from("MCP tool response omitted text content"))?;
    serde_json::from_str(text).map_err(|error| {
        format!(
            "MCP tool {name} content was not JSON ({} bytes): {text:?}: {error}",
            text.len(),
        )
    })
}

fn require_success(response: &Value, operation: &str) -> Result<(), String> {
    if response.get("error").is_some() {
        return Err(format!("{operation} returned an MCP error: {response}"));
    }
    Ok(())
}

fn validate_allocation(value: &Value, config: &RuntimeConfig) -> Result<(), String> {
    for (key, expected) in [
        ("instance_id", config.instance_id.as_str()),
        ("caller_id", config.caller_id.as_str()),
        ("session_id", config.session_id.as_str()),
        ("lease_id", config.lease_id.as_str()),
    ] {
        if value[key].as_str() != Some(expected) {
            return Err(format!("gateway allocation returned unexpected {key}"));
        }
    }
    if value["lease_epoch"].as_u64() != Some(config.lease_epoch) {
        return Err(String::from(
            "gateway allocation returned unexpected lease epoch",
        ));
    }
    Ok(())
}

fn identity_headers(config: &RuntimeConfig, correlation: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("x-sts2-instance-id"),
            config.instance_id.clone(),
        ),
        (String::from("x-sts2-caller-id"), config.caller_id.clone()),
        (String::from("x-sts2-session-id"), config.session_id.clone()),
        (
            String::from("x-mcp-session-id"),
            config.mcp_session_id.clone(),
        ),
        (String::from("x-sts2-lease-id"), config.lease_id.clone()),
        (
            String::from("x-sts2-lease-epoch"),
            config.lease_epoch.to_string(),
        ),
        (
            String::from("x-sts2-correlation-id"),
            String::from(correlation),
        ),
    ])
}

struct McpProcess {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl McpProcess {
    fn spawn(config: &RuntimeConfig) -> Result<Self, String> {
        let mut child = Command::new(&config.mcp_binary)
            .env("STS2_GATEWAY_ADDR", &config.gateway_address)
            .env("STS2_GATEWAY_TOKEN", &config.gateway_token)
            .env("STS2_RUNTIME_PROFILE", &config.runtime_profile)
            .env("STS2_INSTANCE_ID", &config.instance_id)
            .env("STS2_CALLER_ID", &config.caller_id)
            .env("STS2_SESSION_ID", &config.session_id)
            .env("STS2_MCP_SESSION_ID", &config.mcp_session_id)
            .env("STS2_LEASE_ID", &config.lease_id)
            .env("STS2_LEASE_EPOCH", config.lease_epoch.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("MCP process failed to start: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| String::from("MCP process did not expose stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| String::from("MCP process did not expose stdout"))?;
        Ok(Self {
            child,
            input: Some(input),
            output: BufReader::new(output),
        })
    }

    fn call(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| String::from("MCP stdin is closed"))?;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let line = serde_json::to_string(&request)
            .map_err(|error| format!("MCP request serialization failed: {error}"))?;
        input
            .write_all(line.as_bytes())
            .and_then(|_| input.write_all(b"\n"))
            .and_then(|_| input.flush())
            .map_err(|error| format!("MCP request write failed: {error}"))?;
        let mut response = String::new();
        self.output
            .read_line(&mut response)
            .map_err(|error| format!("MCP response read failed: {error}"))?;
        if response.is_empty() || response.len() > MAX_RESPONSE_BYTES {
            return Err(String::from("MCP response was empty or oversized"));
        }
        serde_json::from_str(response.trim())
            .map_err(|error| format!("MCP response was not JSON: {error}"))
    }

    fn close(&mut self) -> Result<(), String> {
        let _ = self.input.take();
        let status = self
            .child
            .wait()
            .map_err(|error| format!("MCP process wait failed: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("MCP process exited with {status}"))
        }
    }
}
