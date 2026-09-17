// SPDX-License-Identifier: MIT

use super::super::super::{INSTANCE, LEASE, LEASE_EPOCH, SESSION};
use serde_json::{Value, json};

pub(super) fn gameplay_state(kind: &str, correlation: &str) -> Value {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))
    .expect("Runtime-v3 state golden is valid");
    response["correlation_id"] = json!(correlation);
    response["instance_id"] = json!(INSTANCE);
    response["session_id"] = json!(SESSION);
    response["lease_id"] = json!(LEASE);
    response["lease_epoch"] = json!(LEASE_EPOCH);
    response["kind"] = json!(kind);
    response["generation"] = json!(0);
    response["state_id"] = json!("combat-1");
    response["observation"]["state_id"] = json!("combat-1");
    response["observation"]["generation"] = json!(0);
    response["observation"]["state"] = json!({"state":"combat","turn_index":1,"enemies":[]});
    response["legal_actions"] = json!([
        {"action_id":"combat.end-turn","action":{"kind":"end_turn"}}
    ]);
    if kind == "legal_actions_response" {
        response["observation"] = Value::Null;
    }
    response
}

pub(super) fn gameplay_dispatch(request: &Value, correlation: &str) -> Result<Value, String> {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))
    .map_err(|_| String::from("Runtime-v3 settled golden could not be read"))?;
    response["correlation_id"] = json!(correlation);
    response["instance_id"] = json!(request["instance_id"]);
    response["session_id"] = json!(request["session_id"]);
    response["lease_id"] = json!(request["lease_id"]);
    response["lease_epoch"] = request["lease_epoch"].clone();
    response["operation_id"] = request["operation_id"].clone();
    response["state_id"] = request["state_id"].clone();
    response["generation"] = json!(1);
    // Runtime-v3 response envelopes keep the submitted action on the request
    // side; a settled response must leave this field null per the schema.
    response["action"] = Value::Null;
    response["observation"]["state_id"] = request["state_id"].clone();
    response["observation"]["generation"] = json!(1);
    response["observation"]["state"] = json!({"state":"victory"});
    response["transition"]["state_id"] = request["state_id"].clone();
    let from_generation = request["generation"]
        .as_u64()
        .unwrap_or(1)
        .saturating_sub(1);
    response["transition"]["from_generation"] = json!(from_generation);
    response["transition"]["to_generation"] = json!(1);
    Ok(response)
}

pub(super) fn gameplay_wait(request: &Value, correlation: &str) -> Result<Value, String> {
    let mut response: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))
    .map_err(|_| String::from("Runtime-v3 wait golden could not be read"))?;
    response["correlation_id"] = json!(correlation);
    response["instance_id"] = json!(request["instance_id"]);
    response["session_id"] = json!(request["session_id"]);
    response["lease_id"] = json!(request["lease_id"]);
    response["lease_epoch"] = request["lease_epoch"].clone();
    response["operation_id"] = request["operation_id"].clone();
    response["state_id"] = json!("combat-1");
    response["generation"] = json!(1);
    response["kind"] = json!("wait_response");
    response["status"] = json!("settled");
    response["wait_for_millis"] = Value::Null;
    response["wait_outcome"] = json!("successor");
    response["action"] = Value::Null;
    response["observation"]["state_id"] = json!("combat-1");
    response["observation"]["generation"] = json!(1);
    response["observation"]["state"] = json!({"state":"victory"});
    let from_generation = request["generation"]
        .as_u64()
        .unwrap_or(1)
        .saturating_sub(1);
    response["transition"]["from_generation"] = json!(from_generation);
    response["transition"]["to_generation"] = json!(1);
    response["transition"]["state_id"] = json!("combat-1");
    response["transition"]["effect_kind"] = json!("end_turn.settled");
    Ok(response)
}
