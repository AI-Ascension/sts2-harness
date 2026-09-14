// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};

#[test]
fn schema_and_conformance_vectors_are_closed_and_executable() {
    let schema: serde_json::Value =
        serde_json::from_slice(SCHEMA).expect("wire schema is valid JSON");
    let defs = schema
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .expect("wire schema has definitions");
    for name in [
        "request_envelope",
        "decision_envelope",
        "decision_request",
        "decision",
    ] {
        assert!(defs.contains_key(name), "missing schema definition {name}");
    }

    let conformance: serde_json::Value =
        serde_json::from_slice(CONFORMANCE).expect("conformance vectors are valid JSON");
    for (section, required) in [
        (
            "request_vectors",
            [
                "standard_at_bound",
                "standard_over_bound",
                "map_at_bound",
                "map_over_bound",
                "envelope_overhead_standard",
                "envelope_overhead_map",
                "wrong_schema",
                "swapped_package",
            ]
            .as_slice(),
        ),
        (
            "decision_vectors",
            ["plan", "action", "wait", "reobserve", "recovery"].as_slice(),
        ),
        (
            "envelope_vectors",
            [
                "wrong_wire_version",
                "wrong_request_id",
                "wrong_turn_id",
                "cancelled",
                "failed",
                "duplicate_field",
                "nested_duplicate_field",
                "nested_unknown_field",
                "trailing_bytes",
                "invalid_utf8",
            ]
            .as_slice(),
        ),
        (
            "capability_vectors",
            [
                "terminal_decision",
                "turn_identity",
                "graceful_eof",
                "idempotency",
                "cancellation",
                "recovery",
            ]
            .as_slice(),
        ),
    ] {
        let vectors = conformance
            .get(section)
            .and_then(serde_json::Value::as_array)
            .expect("conformance section is an array");
        let names = vectors
            .iter()
            .filter_map(|vector| vector.get("name").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        for name in required {
            assert!(names.contains(name), "{section} omitted {name}");
        }
        for vector in vectors {
            assert!(
                vector.get("expected").is_some()
                    || section == "capability_vectors"
                    || section == "decision_vectors",
                "{section} vector has no executable expected result: {vector}"
            );
        }
    }

    let standard = pad_frame(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES);
    assert!(parse_bridge_request(&standard, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
    let map = pad_frame(
        &exo_contract_map::map_request_bytes(),
        EXO_MAX_MAP_REQUEST_BYTES,
    );
    assert!(parse_bridge_request(&map, EXO_MAX_MAP_REQUEST_BYTES).is_ok());
    for decision in [
        br#"{"decision":"plan","action_ids":["combat.end-turn"],"rationale":"plan"}"#.as_slice(),
        DECISION,
        br#"{"decision":"wait","rationale":"wait"}"#.as_slice(),
        br#"{"decision":"reobserve","rationale":"reobserve"}"#.as_slice(),
        br#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"recover"}"#.as_slice(),
    ] {
        assert!(parse_bridge_decision(decision).is_ok());
    }

    let mut wrong_schema: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    wrong_schema["schema"] = json!("wrong-schema-v0");
    let wrong_schema = serde_json::to_vec(&wrong_schema).expect("wrong schema serializes");
    assert_eq!(
        parse_bridge_request(&wrong_schema, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
    );

    let failed = encode_bridge_response(
        "request-failed",
        "turn-failed",
        ExoWireOutcome::Failed,
        None,
        Some("remote_failure"),
    )
    .expect("failed response encodes");
    assert_eq!(
        parse_bridge_decision_envelope(&failed, "request-failed", "turn-failed"),
        Err(ExoWireError::RemoteFailure)
    );
}

#[test]
fn schema_variants_and_optional_outcomes_match_rust_parser() {
    let schema: serde_json::Value =
        serde_json::from_slice(SCHEMA).expect("wire schema is valid JSON");
    let defs = schema
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .expect("wire schema has definitions");
    let decision_refs = defs["decision"]["oneOf"]
        .as_array()
        .expect("decision schema is a closed variant union")
        .iter()
        .filter_map(|variant| variant["$ref"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        decision_refs,
        vec![
            "#/$defs/plan_decision",
            "#/$defs/action_decision",
            "#/$defs/wait_decision",
            "#/$defs/reobserve_decision",
            "#/$defs/recovery_decision",
        ]
    );
    assert_eq!(
        defs["decision_envelope"]["required"],
        json!(["wire_version", "request_id", "turn_id", "outcome"])
    );
    for (decision, expected) in [
        (
            br#"{"decision":"wait","action_id":"combat.end-turn","rationale":"wait"}"#.as_slice(),
            false,
        ),
        (
            br#"{"decision":"action","rationale":"missing action"}"#.as_slice(),
            false,
        ),
        (
            br#"{"decision":"recovery","recovery_kind":"reconcile","rationale":"missing operation"}"#
                .as_slice(),
            false,
        ),
        (
            br#"{"decision":"recovery","recovery_kind":"reobserve","operation_id":"op-1","rationale":"extra operation"}"#
                .as_slice(),
            false,
        ),
        (
            br#"{"decision":"wait","rationale":"wait"}"#.as_slice(),
            true,
        ),
        (
            br#"{"decision":"recovery","recovery_kind":"reconcile","operation_id":"op-1","rationale":"reconcile"}"#
                .as_slice(),
            true,
        ),
        (
            r#"{"decision":"wait","rationale":"é"}"#.as_bytes(),
            false,
        ),
    ] {
        assert_eq!(parse_bridge_decision(decision).is_ok(), expected);
    }
    let cancelled_without_optional_fields =
        br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","outcome":"cancelled"}"#;
    assert_eq!(
        parse_bridge_decision_envelope(cancelled_without_optional_fields, "request-1", "turn-1"),
        Err(ExoWireError::Cancelled)
    );
    let failed_without_error =
        br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","outcome":"failed"}"#;
    assert_eq!(
        parse_bridge_decision_envelope(failed_without_error, "request-1", "turn-1"),
        Err(ExoWireError::InvalidShape)
    );
    let decision_without_decision =
        br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","outcome":"decision","error_code":null}"#;
    assert_eq!(
        parse_bridge_decision_envelope(decision_without_decision, "request-1", "turn-1"),
        Err(ExoWireError::InvalidShape)
    );
}

#[test]
fn exact_request_bounds_and_envelope_overhead_are_enforced() {
    let standard = pad_frame(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES);
    assert_eq!(standard.len(), EXO_MAX_STANDARD_REQUEST_BYTES);
    assert!(parse_bridge_request(&standard, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
    let mut standard_over = standard.clone();
    standard_over.push(b' ');
    assert_eq!(
        parse_bridge_request(&standard_over, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::TooLarge)
    );

    let map_source = exo_contract_map::map_request_bytes();
    let map = pad_frame(&map_source, EXO_MAX_MAP_REQUEST_BYTES);
    assert_eq!(map.len(), EXO_MAX_MAP_REQUEST_BYTES);
    assert!(parse_bridge_request(&map, EXO_MAX_MAP_REQUEST_BYTES).is_ok());
    let mut map_over = map.clone();
    map_over.push(b' ');
    assert_eq!(
        parse_bridge_request(&map_over, EXO_MAX_MAP_REQUEST_BYTES),
        Err(ExoWireError::TooLarge)
    );

    for (inner, limit) in [
        (standard, EXO_MAX_STANDARD_REQUEST_BYTES),
        (map, EXO_MAX_MAP_REQUEST_BYTES),
    ] {
        let mut envelope = br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","request":"#
            .to_vec();
        envelope.extend_from_slice(&inner);
        envelope.push(b'}');
        assert!(envelope.len() > limit);
        assert_eq!(
            parse_bridge_request_envelope(&envelope, limit),
            Err(ExoWireError::TooLarge)
        );
    }
}

#[test]
fn schema_nested_nulls_and_rust_request_validation_have_parity() {
    let schema: serde_json::Value =
        serde_json::from_slice(SCHEMA).expect("wire schema is valid JSON");
    let defs = schema["$defs"]
        .as_object()
        .expect("wire schema definitions");
    assert_eq!(
        defs["standard_observation"]["additionalProperties"],
        json!(false)
    );
    assert_eq!(defs["map_snapshot"]["additionalProperties"], json!(false));
    assert_eq!(
        defs["map_node"]["additionalProperties"],
        json!(false),
        "map node fields stay constrained by the production validator"
    );
    assert!(
        defs["map_snapshot"]["properties"]["map_instance_id"]["$ref"]
            .as_str()
            .is_some(),
        "map identity null is rejected by both schema and parser"
    );
    assert!(
        defs["decision_request"]["properties"]["map_context"]["anyOf"]
            .as_array()
            .expect("map context alternatives")
            .iter()
            .any(|item| item == &json!({"type": "null"})),
        "explicit null map context is the Rust Option::None shape"
    );
    assert!(
        defs["decision_request"]["properties"]["management_profile"]["anyOf"]
            .as_array()
            .expect("management profile alternatives")
            .iter()
            .any(|item| item == &json!({"type": "null"})),
        "explicit null management profile is the Rust Option::None shape"
    );

    let mut standard: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    standard["map_context"] = Value::Null;
    standard["management_profile"] = Value::Null;
    standard["management_context"] = Value::Null;
    let standard = serde_json::to_vec(&standard).expect("null optionals serialize");
    assert!(parse_bridge_request(&standard, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());

    let mut invalid_observation: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    invalid_observation["observation"]["player"]["unexpected"] = json!(true);
    let invalid_observation =
        serde_json::to_vec(&invalid_observation).expect("invalid observation serializes");
    assert_eq!(
        parse_bridge_request(&invalid_observation, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
    );

    let mut invalid_map: serde_json::Value =
        serde_json::from_slice(&exo_contract_map::map_request_bytes())
            .expect("generated map request is JSON");
    invalid_map["map_context"]["snapshot"]["nodes"][0]["unexpected"] = json!(true);
    let invalid_map = serde_json::to_vec(&invalid_map).expect("invalid map serializes");
    assert_eq!(
        parse_bridge_request(&invalid_map, EXO_MAX_MAP_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
    );

    let mut null_map_identity: serde_json::Value =
        serde_json::from_slice(&exo_contract_map::map_request_bytes())
            .expect("generated map request is JSON");
    null_map_identity["map_context"]["snapshot"]["map_instance_id"] = Value::Null;
    let null_map_identity =
        serde_json::to_vec(&null_map_identity).expect("null map identity serializes");
    assert_eq!(
        parse_bridge_request(&null_map_identity, EXO_MAX_MAP_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
    );
}
