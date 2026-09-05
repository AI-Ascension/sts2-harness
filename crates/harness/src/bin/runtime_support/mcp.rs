// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::config::RuntimeConfig;
use super::http::GatewayClient;

use super::mcp_process::McpProcess;
use super::response_validation::validate_response;

#[path = "trace_runtime_v1.rs"]
mod trace_runtime_v1;
#[path = "trace_runtime_v2.rs"]
mod trace_runtime_v2;

fn run_trace(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
    match config.runtime_profile.as_str() {
        "runtime-v1" => trace_runtime_v1::run(mcp, config),
        "runtime-v2" => trace_runtime_v2::run(mcp, config),
        _ => Err(String::from("unsupported runtime profile")),
    }
}

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
    );
    validate_or_release_allocation(allocation, &config, |headers| {
        client.request(
            "POST",
            &format!("/v1/instances/{}/release", config.instance_id),
            &Value::Null,
            headers,
        )
    })?;
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
    let failures: Vec<_> = [trace_result, close_result, release_result.map(|_| ())]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn wait_for_v2_player_turn(
    mcp: &mut McpProcess,
    config: &RuntimeConfig,
    request_id: &mut u64,
    request_ids: &mut Vec<u64>,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(config.wait_for_combat_seconds);
    loop {
        request_ids.push(*request_id);
        let observation = tool_call(
            mcp,
            config,
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
            return Err(String::from(
                "Runtime-v2 host did not reach combat/player_turn",
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn wait_for_operation_settlement(
    mcp: &mut McpProcess,
    config: &RuntimeConfig,
    request_id: &mut u64,
    request_ids: &mut Vec<u64>,
    operation_id: &str,
    generation: u64,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(config.settlement_timeout_seconds);
    loop {
        request_ids.push(*request_id);
        let reconciled = tool_call(
            mcp,
            config,
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
            return Err(String::from(
                "Runtime operation did not settle before the bounded timeout; outcome remains uncertain",
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn trace_lineage(config: &RuntimeConfig) -> Value {
    json!({
        "instance_id": config.instance_id,
        "gateway_session_id": config.session_id,
        "mcp_session_id": config.mcp_session_id,
        "lease_id": config.lease_id,
        "lease_epoch": config.lease_epoch,
        "run_id": config.run_id,
        "episode_id": config.episode_id,
        "trajectory_id": config.trajectory_id,
        "artifact_id": config.artifact_id,
    })
}

fn trace_correlations(entries: &[(&str, &Value)]) -> Result<Value, String> {
    let mut correlations = Map::new();
    for (label, value) in entries {
        let correlation_id = value["correlation_id"]
            .as_str()
            .ok_or_else(|| format!("trace response {label} omitted its correlation identity"))?;
        correlations.insert(
            String::from(*label),
            Value::String(String::from(correlation_id)),
        );
    }
    Ok(Value::Object(correlations))
}

fn require_kind(value: &Value, expected: &str) -> Result<(), String> {
    if value["kind"] == expected {
        Ok(())
    } else {
        Err(format!("Runtime-v2 response kind was not {expected}"))
    }
}

fn tool_call(
    mcp: &mut McpProcess,
    config: &RuntimeConfig,
    id: u64,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let response = mcp.call(
        id,
        "tools/call",
        json!({"name": name, "arguments": &arguments}),
    )?;
    if response.get("error").is_some() {
        return Err(String::from("MCP tool returned an RPC error"));
    }
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .ok_or_else(|| String::from("MCP tool response omitted text content"))?;
    let value =
        serde_json::from_str(text).map_err(|_| format!("MCP tool {name} content was not JSON"))?;
    validate_response(&value, config, id, name, &arguments)?;
    super::v1_projection::validate_error_flag(&response, &value)?;
    Ok(value)
}

fn require_success(response: &Value, operation: &str) -> Result<(), String> {
    if response.get("error").is_some() || response["result"]["isError"] == true {
        return Err(format!("{operation} returned an MCP error"));
    }
    if !response["result"].is_object() {
        return Err(format!("{operation} omitted its MCP result"));
    }
    Ok(())
}

#[path = "allocation_cleanup.rs"]
mod allocation_cleanup;
pub(super) use allocation_cleanup::validate_or_release_allocation;

pub(super) fn validate_allocation(value: &Value, config: &RuntimeConfig) -> Result<(), String> {
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

#[cfg(test)]
#[path = "allocation_tests.rs"]
mod allocation_tests;

#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn errors_are_redacted_and_tool_errors_are_not_success() {
        for value in [
            json!({"error":{"message":"secret-marker"}}),
            json!({"result":{"isError":true,"content":[{"text":"secret-marker"}]}}),
        ] {
            assert_eq!(
                require_success(&value, "get_state"),
                Err("get_state returned an MCP error".into())
            );
        }
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
