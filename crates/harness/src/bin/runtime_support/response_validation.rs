// SPDX-License-Identifier: MIT

use super::config::RuntimeConfig;
use serde_json::Value;

pub(super) fn validate_response(
    value: &Value,
    config: &RuntimeConfig,
    id: u64,
    name: &str,
    arguments: &Value,
) -> Result<(), String> {
    if config.runtime_profile == "runtime-v1" {
        return super::v1_projection::validate(value, name, arguments);
    }
    validate_profile(value, config)?;
    validate_identity(value, config, id)?;
    let kind = response_kind(name)?;
    if value["kind"] != kind {
        return Err(String::from("MCP tool response kind was invalid"));
    }
    validate_runtime_v2(value, config)?;
    validate_operation(value, config, arguments)?;
    validate_action(value, arguments)
}

fn validate_profile(value: &Value, config: &RuntimeConfig) -> Result<(), String> {
    if value["protocol_version"] != config.runtime_profile {
        return Err(String::from(
            "MCP tool response protocol did not match the selected profile",
        ));
    }
    if config.runtime_profile == "runtime-v1"
        && value["schema_digest"]
            != "a76086d7a68668fd4cff53999369d2b450b0d6623827393882f458f2aa1f93eb"
    {
        return Err(String::from(
            "MCP tool response violated the Runtime-v1 artifact identity",
        ));
    }
    Ok(())
}

fn validate_identity(value: &Value, config: &RuntimeConfig, id: u64) -> Result<(), String> {
    for (key, expected) in [
        ("instance_id", config.instance_id.as_str()),
        ("session_id", config.session_id.as_str()),
        ("lease_id", config.lease_id.as_str()),
    ] {
        if value[key].as_str() != Some(expected) {
            return Err(format!("MCP tool response mismatched {key}"));
        }
    }
    if value["lease_epoch"].as_u64() != Some(config.lease_epoch)
        || value["correlation_id"].as_str() != Some(id.to_string().as_str())
        || value["generation"]
            .as_u64()
            .is_none_or(|generation| generation > 9_007_199_254_740_991)
    {
        return Err(String::from(
            "MCP tool response fence or correlation was invalid",
        ));
    }
    Ok(())
}

fn response_kind(name: &str) -> Result<&'static str, String> {
    match name {
        "get_state" => Ok("state_response"),
        "submit_action" => Ok("action_response"),
        "reconcile_action" => Ok("reconcile_response"),
        _ => Err(String::from("unsupported runtime tool")),
    }
}

fn validate_runtime_v2(value: &Value, config: &RuntimeConfig) -> Result<(), String> {
    if config.runtime_profile == "runtime-v2" {
        sts2_harness::RuntimeV2Message::from_json(&value.to_string())
            .map_err(|_| String::from("MCP tool response violated the Runtime-v2 contract"))?;
    }
    Ok(())
}

fn validate_operation(
    value: &Value,
    config: &RuntimeConfig,
    arguments: &Value,
) -> Result<(), String> {
    if config.runtime_profile != "runtime-v1"
        && let Some(operation) = arguments.get("operation_id")
        && value.get("operation_id") != Some(operation)
    {
        return Err(String::from(
            "MCP tool response operation did not match the request",
        ));
    }
    Ok(())
}

fn validate_action(value: &Value, arguments: &Value) -> Result<(), String> {
    if let Some(action) = arguments.get("action_id") {
        let returned = &value["action"]["action_id"];
        if returned != action {
            return Err(String::from(
                "MCP tool response action did not match the request",
            ));
        }
    }
    for key in ["card_index", "target_id"] {
        if let Some(expected) = arguments.get(key)
            && value["action"].get(key) != Some(expected)
        {
            return Err(String::from(
                "MCP tool response action payload did not match the request",
            ));
        }
    }
    validate_action_payload(value, arguments)
}

fn validate_action_payload(value: &Value, arguments: &Value) -> Result<(), String> {
    for key in ["card_index", "target_id"] {
        if let Some(expected) = arguments.get(key)
            && value["action"].get(key) != Some(expected)
        {
            return Err(String::from(
                "MCP tool response action payload did not match the request",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> RuntimeConfig {
        RuntimeConfig {
            gateway_address: "127.0.0.1:1".into(),
            gateway_token: "synthetic".into(),
            mcp_binary: "unused".into(),
            runtime_profile: "runtime-v2".into(),
            instance_id: "instance-1".into(),
            caller_id: "harness".into(),
            session_id: "session-1".into(),
            lease_id: "lease-1".into(),
            lease_epoch: 1,
            mcp_session_id: "separate-mcp-session".into(),
            run_id: "run-1".into(),
            episode_id: "episode-1".into(),
            trajectory_id: "trajectory-1".into(),
            artifact_id: "artifact-1".into(),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 0,
        }
    }

    #[test]
    fn runtime_v2_response_is_correlated_and_validated_before_trace_use() -> Result<(), String> {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v2/golden/legal-action-settled.json"
        ))
        .map_err(|_| "fixture failed")?;
        value["correlation_id"] = json!("4");
        let args = json!({"operation_id":"op-1","action_id":"end_turn"});
        validate_response(&value, &config(), 4, "submit_action", &args)?;
        for (field, bad) in [
            ("instance_id", json!("foreign-instance")),
            ("session_id", json!("separate-mcp-session")),
            ("lease_id", json!("foreign-lease")),
            ("lease_epoch", json!(2)),
            ("correlation_id", json!("3")),
            ("operation_id", json!("foreign-operation")),
            ("protocol_version", json!("runtime-v3")),
            ("schema_digest", json!("a".repeat(64))),
            ("generation", json!(u64::MAX)),
        ] {
            let mut invalid = value.clone();
            invalid[field] = bad;
            assert!(validate_response(&invalid, &config(), 4, "submit_action", &args).is_err());
        }
        value["unexpected_private_field"] = json!("sensitive-marker");
        let error = validate_response(&value, &config(), 4, "submit_action", &args)
            .err()
            .ok_or("unknown field accepted")?;
        assert!(!error.contains("sensitive-marker"));
        Ok(())
    }
}
