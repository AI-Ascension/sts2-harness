// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::config::RuntimeConfig;
use super::http::GatewayClient;

pub(super) use super::mcp_process::McpProcess;

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
        BTreeMap::new(),
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

fn run_trace(mcp: &mut McpProcess, config: &RuntimeConfig) -> Result<(), String> {
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
            "accepted_effect": {"kind":"status_overlay_visible","generation":generation + 1},
            "stale_rejection": stale["error_code"],
            "observation": {"overlay_visible":true,"action_count":1}
        }))
        .map_err(|error| format!("trace serialization failed: {error}"))?
    );
    Ok(())
}

fn tool_call(mcp: &mut McpProcess, id: u64, name: &str, arguments: Value) -> Result<Value, String> {
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
        serde_json::from_str(text).map_err(|_| String::from("MCP tool content was not JSON"))?;
    super::v1_projection::validate(&value, name, &arguments)?;
    super::v1_projection::validate_error_flag(&response, &value)?;
    Ok(value)
}

fn require_success(response: &Value, operation: &str) -> Result<(), String> {
    if response.get("error").is_some() || response["result"]["isError"] == true {
        return Err(format!("{operation} returned an MCP error"));
    }
    Ok(())
}

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

pub(super) fn identity_headers(
    config: &RuntimeConfig,
    correlation: &str,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("x-sts2-instance-id"),
            config.instance_id.clone(),
        ),
        (String::from("x-sts2-caller-id"), config.caller_id.clone()),
        (String::from("x-sts2-session-id"), config.session_id.clone()),
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
mod tests {
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
