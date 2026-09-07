// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    RuntimeV4ExpertActionRequest, RuntimeV4ExpertActionResult, RuntimeV4ExpertActionStatus,
    verify_runtime_v4_expert_action_artifact,
};

fn request_value() -> Value {
    serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-action/golden/action-request.json"
    )))
    .unwrap_or(Value::Null)
}

fn settled_value() -> Value {
    let mut settled: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
    )))
    .unwrap_or(Value::Null);
    let mut observation: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))
    .unwrap_or(Value::Null);
    observation["generation"] = json!(8);
    observation["state_id"] = json!("live:8");
    settled["correlation_id"] = json!("request-1");
    settled["observation"] = observation;
    settled
}

#[test]
fn action_artifact_and_fresh_settlement_reach_the_harness_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    verify_runtime_v4_expert_action_artifact()?;
    let request = RuntimeV4ExpertActionRequest::from_value(request_value())?;
    let result = RuntimeV4ExpertActionResult::from_value(settled_value())?;
    result.matches_request(&request)?;
    assert_eq!(result.status(), RuntimeV4ExpertActionStatus::Settled);
    assert!(result.is_settled());
    assert_eq!(result.operation_id(), request.operation_id());
    assert_eq!(result.generation(), 8);
    Ok(())
}

#[test]
fn unknown_outcome_is_reconciled_without_a_second_action_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let request = RuntimeV4ExpertActionRequest::from_value(request_value())?;
    let mut unknown = request_value();
    unknown["kind"] = json!("action_response");
    unknown["status"] = json!("unknown");
    unknown["error_code"] = json!("transport_timeout");
    let result = RuntimeV4ExpertActionResult::from_value(unknown)?;
    result.matches_request(&request)?;
    assert_eq!(result.status(), RuntimeV4ExpertActionStatus::Unknown);
    Ok(())
}

#[test]
fn action_parser_rejects_duplicate_keys_and_foreign_operations() {
    let duplicate = br#"{"protocol_version":"runtime-v4-expert-action","protocol_version":"runtime-v4-expert-action"}"#;
    assert!(RuntimeV4ExpertActionRequest::parse(duplicate).is_err());
    let mut foreign = settled_value();
    foreign["operation_id"] = json!("other-operation");
    let request = RuntimeV4ExpertActionRequest::from_value(request_value()).unwrap();
    let result = RuntimeV4ExpertActionResult::from_value(foreign).unwrap();
    assert!(result.matches_request(&request).is_err());
}
