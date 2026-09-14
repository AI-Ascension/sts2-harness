// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};

#[test]
fn json_schema_validator_executes_request_decision_and_envelope_vectors() {
    let schema: serde_json::Value =
        serde_json::from_slice(SCHEMA).expect("wire schema is valid JSON");
    let capability_validator = jsonschema::validator_for(&schema).expect("capability schema");
    let capability: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../protocol-artifact/exo-bridge-v1/golden/capability-source.json"
    ))
    .expect("capability fixture");
    assert!(capability_validator.is_valid(&capability));

    let request_validator = definition_validator(&schema, "decision_request");
    let request_envelope_validator = definition_validator(&schema, "request_envelope");
    let decision_validator = definition_validator(&schema, "decision");
    let decision_envelope_validator = definition_validator(&schema, "decision_envelope");
    let request: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    assert!(request_validator.is_valid(&request));
    let map_request: serde_json::Value =
        serde_json::from_slice(&exo_contract_map::map_request_bytes())
            .expect("generated map request is JSON");
    assert!(request_validator.is_valid(&map_request));

    // The production parser stores both management fields as `Option`s, so
    // explicit JSON null and omission are equivalent.  Keep every pairing
    // executable here rather than relying on schema definition shape checks.
    assert!(request_validator.is_valid(&request));
    let mut explicit_null_pair = request.clone();
    explicit_null_pair["management_profile"] = Value::Null;
    explicit_null_pair["management_context"] = Value::Null;
    assert!(request_validator.is_valid(&explicit_null_pair));
    let mut enabled_pair = request.clone();
    enabled_pair["management_profile"] = json!("management-enabled");
    enabled_pair["management_context"] = json!({"scope": "management"});
    assert!(request_validator.is_valid(&enabled_pair));
    let mut profile_without_context = request.clone();
    profile_without_context["management_profile"] = json!("management-enabled");
    profile_without_context["management_context"] = Value::Null;
    assert!(!request_validator.is_valid(&profile_without_context));
    let mut context_without_profile = request.clone();
    context_without_profile["management_context"] = json!({"scope": "management"});
    assert!(!request_validator.is_valid(&context_without_profile));
    let mut null_profile_with_context = request.clone();
    null_profile_with_context["management_profile"] = Value::Null;
    null_profile_with_context["management_context"] = json!({"scope": "management"});
    assert!(!request_validator.is_valid(&null_profile_with_context));
    let mut null_context_with_profile = request.clone();
    null_context_with_profile["management_profile"] = json!("management-enabled");
    null_context_with_profile["management_context"] = Value::Null;
    assert!(!request_validator.is_valid(&null_context_with_profile));

    let mut unknown = request.clone();
    unknown["unknown"] = json!("reject");
    assert!(!request_validator.is_valid(&unknown));
    let mut invalid_nested = request.clone();
    invalid_nested["observation"]["player"]["unexpected"] = json!(true);
    assert!(!request_validator.is_valid(&invalid_nested));

    let action_validator = definition_validator(&schema, "action");
    assert!(action_validator.is_valid(&json!({
        "kind": "shop_remove",
        "card_id": "card-1"
    })));
    assert!(!action_validator.is_valid(&json!({
        "kind": "shop_remove",
        "item_id": "item-1"
    })));
    assert!(action_validator.is_valid(&json!({
        "kind": "shop_purchase",
        "item_id": "item-1"
    })));
    assert!(!action_validator.is_valid(&json!({
        "kind": "shop_purchase",
        "card_id": "card-1"
    })));

    let parsed_request = parse_bridge_request(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES)
        .expect("golden request parses");
    let request_envelope = encode_bridge_request(
        "request-schema",
        "turn-schema",
        &parsed_request,
        EXO_MAX_STANDARD_REQUEST_BYTES,
    )
    .expect("request envelope encodes");
    let request_envelope: serde_json::Value =
        serde_json::from_slice(&request_envelope).expect("request envelope JSON");
    assert!(request_envelope_validator.is_valid(&request_envelope));

    for decision in [
        json!({"decision": "plan", "action_ids": ["combat.end-turn"], "rationale": "plan"}),
        serde_json::from_slice(DECISION).expect("action decision JSON"),
        json!({"decision": "wait", "rationale": "wait"}),
        json!({"decision": "reobserve", "rationale": "reobserve"}),
        json!({"decision": "recovery", "recovery_kind": "reobserve", "rationale": "recover"}),
    ] {
        assert!(
            decision_validator.is_valid(&decision),
            "decision schema rejected {decision}"
        );
    }
    let decision_response = encode_bridge_response(
        "request-schema",
        "turn-schema",
        ExoWireOutcome::Decision,
        Some(DECISION),
        None,
    )
    .expect("decision response encodes");
    let decision_response: serde_json::Value =
        serde_json::from_slice(&decision_response).expect("decision response JSON");
    assert!(decision_envelope_validator.is_valid(&decision_response));
    let cancelled_response = encode_bridge_response(
        "request-schema",
        "turn-schema",
        ExoWireOutcome::Cancelled,
        None,
        None,
    )
    .expect("cancelled response encodes");
    let cancelled_response: serde_json::Value =
        serde_json::from_slice(&cancelled_response).expect("cancelled response JSON");
    assert!(decision_envelope_validator.is_valid(&cancelled_response));
    let failed_response = encode_bridge_response(
        "request-schema",
        "turn-schema",
        ExoWireOutcome::Failed,
        None,
        Some("remote_failure"),
    )
    .expect("failed response encodes");
    let failed_response: serde_json::Value =
        serde_json::from_slice(&failed_response).expect("failed response JSON");
    assert!(decision_envelope_validator.is_valid(&failed_response));
}

#[test]
fn encoding_uses_request_profile_limit_for_outer_envelope() {
    let mut request: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    // The fixed 230-byte action IDs leave enough room for a binary search over
    // the bounded objective text, producing a deterministic request just below
    // the standard ceiling without repeatedly serializing hundreds of frames.
    let ids = (0..256)
        .map(|index| format!("a{index:03}{}", "b".repeat(226)))
        .collect::<Vec<_>>();
    let actions = ids
        .iter()
        .map(|id| json!({"action_id": id, "action": {"kind": "end_turn"}}))
        .collect::<Vec<_>>();
    request["legal_action_ids"] = json!(ids);
    request["observation"]["legal_actions"] = json!(actions);
    let mut low = 7usize;
    let mut high = 512usize;
    let mut near_limit = None;
    while low <= high {
        let objective_len = low + (high - low) / 2;
        request["objective"] = Value::String("x".repeat(objective_len));
        let candidate = serde_json::to_vec(&request).expect("near-limit request serializes");
        if candidate.len() <= EXO_MAX_STANDARD_REQUEST_BYTES {
            near_limit = Some(candidate);
            low = objective_len + 1;
        } else {
            high = objective_len.saturating_sub(1);
        }
    }
    let bytes = near_limit.expect("fixture can reach the standard profile ceiling");
    let candidate_value: Value =
        serde_json::from_slice(&bytes).expect("near-limit request is JSON");
    let envelope = json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "request-regression",
        "turn_id": "turn-regression",
        "request": candidate_value
    });
    assert!(
        serde_json::to_vec(&envelope)
            .expect("near-limit envelope serializes")
            .len()
            > EXO_MAX_STANDARD_REQUEST_BYTES
    );
    let request = parse_bridge_request(&bytes, EXO_MAX_STANDARD_REQUEST_BYTES)
        .expect("near-limit standard request parses");
    assert!(request.encode(EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
    assert_eq!(
        encode_bridge_request(
            "request-regression",
            "turn-regression",
            &request,
            EXO_MAX_MAP_REQUEST_BYTES
        ),
        Err(ExoWireError::TooLarge),
        "outer envelope overhead must use the decoded standard profile ceiling"
    );
}

fn definition_validator(schema: &serde_json::Value, name: &str) -> jsonschema::Validator {
    let wrapper = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": schema["$defs"].clone(),
        "$ref": format!("#/$defs/{name}")
    });
    jsonschema::validator_for(&wrapper).expect("definition schema compiles")
}
