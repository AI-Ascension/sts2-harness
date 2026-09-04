// SPDX-License-Identifier: MIT

use serde_json::Value;

// Runtime-v1 MCP intentionally omits the gateway envelope after validating its fence.
// Validate that projected tool contract, not the full downstream wire envelope.
pub(super) fn validate(value: &Value, name: &str, arguments: &Value) -> Result<(), String> {
    let fields: &[&str] = match (name, value["kind"].as_str(), value["status"].as_str()) {
        ("get_state", Some("state_response"), None) => &["kind", "generation", "observation"],
        ("submit_action", Some("action_response"), Some("accepted")) => &[
            "kind",
            "generation",
            "observation",
            "action",
            "status",
            "effect_witness",
        ],
        ("submit_action", Some("action_response"), Some("rejected")) => &[
            "kind",
            "generation",
            "observation",
            "action",
            "status",
            "error_code",
        ],
        _ => {
            return Err(String::from(
                "Runtime-v1 projected response shape is invalid",
            ));
        }
    };
    exact_fields(value, fields)?;
    let generation = value["generation"]
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or("Runtime-v1 projected generation is invalid")?;
    let observation = &value["observation"];
    exact_fields(
        observation,
        &["host_ready", "overlay_visible", "screen", "action_count"],
    )?;
    if observation["host_ready"].as_bool().is_none()
        || observation["overlay_visible"].as_bool().is_none()
        || !safe_text(&observation["screen"], 64)
        || observation["action_count"]
            .as_u64()
            .is_none_or(|value| value > 1024)
    {
        return Err(String::from("Runtime-v1 projected observation is invalid"));
    }
    if name == "submit_action" {
        exact_fields(&value["action"], &["action_id"])?;
        if value["action"]["action_id"] != "show_runtime_probe"
            || value["action"]["action_id"] != arguments["action_id"]
        {
            return Err(String::from(
                "Runtime-v1 projected action does not match the request",
            ));
        }
        if value["status"] == "accepted" {
            exact_fields(&value["effect_witness"], &["kind", "generation"])?;
            if value["effect_witness"]["kind"] != "status_overlay_visible"
                || value["effect_witness"]["generation"].as_u64() != Some(generation)
            {
                return Err(String::from("Runtime-v1 projected witness is invalid"));
            }
        } else if !safe_text(&value["error_code"], 128) {
            return Err(String::from("Runtime-v1 projected rejection is invalid"));
        }
    }
    Ok(())
}

pub(super) fn validate_error_flag(response: &Value, value: &Value) -> Result<(), String> {
    match response["result"].get("isError") {
        None | Some(Value::Bool(false)) => Ok(()),
        Some(Value::Bool(true))
            if value["kind"] == "action_response" && value["status"] == "rejected" =>
        {
            Ok(())
        }
        _ => Err(String::from(
            "MCP tool error is not a validated Runtime-v1 rejection",
        )),
    }
}

fn exact_fields(value: &Value, fields: &[&str]) -> Result<(), String> {
    if value.as_object().is_none_or(|object| {
        object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field))
    }) {
        return Err(String::from(
            "Runtime-v1 projection has missing or unknown fields",
        ));
    }
    Ok(())
}

fn safe_text(value: &Value, max: usize) -> bool {
    value.as_str().is_some_and(|value| {
        !value.is_empty()
            && value.len() <= max
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_the_actual_legacy_projection_and_typed_stale_error() -> Result<(), String> {
        let observation =
            json!({"host_ready":true,"overlay_visible":true,"screen":"host","action_count":1});
        let state = json!({"kind":"state_response","generation":1,"observation":observation});
        validate(&state, "get_state", &json!({}))?;
        let args = json!({"action_id":"show_runtime_probe"});
        let mut action = json!({"kind":"action_response","generation":1,"observation":observation,
            "action":{"action_id":"show_runtime_probe"},"status":"accepted",
            "effect_witness":{"kind":"status_overlay_visible","generation":1}});
        validate(&action, "submit_action", &args)?;
        action["effect_witness"]["generation"] = json!(2);
        assert!(validate(&action, "submit_action", &args).is_err());
        let rejected = json!({"kind":"action_response","generation":1,"observation":observation,
            "action":{"action_id":"show_runtime_probe"},"status":"rejected",
            "error_code":"sts2.game-mod/stale_generation"});
        validate(&rejected, "submit_action", &args)?;
        let error_response = json!({"result":{"isError":true}});
        validate_error_flag(&error_response, &rejected)?;
        assert!(validate_error_flag(&error_response, &state).is_err());
        assert!(validate(&json!({"error":"secret-marker"}), "submit_action", &args).is_err());
        let mut extra = rejected.clone();
        extra["private"] = json!("secret-marker");
        let error = validate(&extra, "submit_action", &args)
            .err()
            .ok_or("unexpected field accepted")?;
        assert!(!error.contains("secret-marker"));
        Ok(())
    }
}
