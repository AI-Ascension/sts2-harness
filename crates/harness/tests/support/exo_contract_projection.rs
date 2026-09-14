// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};

/// Builds the harness fair-play expert projection: the canonical Runtime-v4 expert observation
/// with the pinned `harness_projection` marker and the REST selector's projected legal actions.
fn projected_expert_request() -> Value {
    let mut observation: Value = serde_json::from_slice(include_bytes!(
        "../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ))
    .expect("golden expert observation is JSON");
    observation["harness_projection"] = json!("sts2-harness/runtime-v4-expert-fair-play-v1");
    observation["legal_actions"] = json!([
        {"action_id": "select:7:card:1", "action": {"kind": "select_card", "card_id": "card:1"}},
        {"action_id": "select:7:player:local", "action": {"kind": "select_player", "player_id": "player:local"}},
        {"action_id": "confirm:7", "action": {"kind": "confirm_selection"}},
        {"action_id": "cancel:7", "action": {"kind": "cancel_selection"}}
    ]);
    json!({
        "schema": "sts2.exo-decision-v1",
        "provider_revision": "b06869ab789dee3f80ca474b5fa89dbe47ccb859",
        "model_execution_id": "execution-projection",
        "state_id": "live:7",
        "generation": 7,
        "observation": observation,
        "legal_action_ids": [
            "select:7:card:1",
            "select:7:player:local",
            "confirm:7",
            "cancel:7"
        ],
        "objective": "decide",
        "hard_constraints": [],
        "max_response_bytes": 8192
    })
}

#[test]
fn harness_projection_request_and_envelope_have_schema_parser_parity() {
    let schema: serde_json::Value =
        serde_json::from_slice(SCHEMA).expect("wire schema is valid JSON");
    let observation_validator =
        exo_contract_schema_vectors::definition_validator(&schema, "observation");
    let projection_validator = exo_contract_schema_vectors::definition_validator(
        &schema,
        "harness_projection_observation",
    );
    let request_validator =
        exo_contract_schema_vectors::definition_validator(&schema, "decision_request");
    let envelope_validator =
        exo_contract_schema_vectors::definition_validator(&schema, "request_envelope");

    // The canonical expert observation stays in the unmarked expert variant.
    let canonical: Value = serde_json::from_slice(include_bytes!(
        "../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ))
    .expect("golden expert observation is JSON");
    let canonical_request = json!({
        "schema": "sts2.exo-decision-v1",
        "provider_revision": "b06869ab789dee3f80ca474b5fa89dbe47ccb859",
        "model_execution_id": "execution-expert",
        "state_id": "live:7",
        "generation": 7,
        "observation": canonical,
        "legal_action_ids": [
            "play:7:card:1:enemy:1",
            "potion:7:potion:fire:enemy:1",
            "end:7"
        ],
        "objective": "decide",
        "hard_constraints": [],
        "max_response_bytes": 8192
    });
    assert!(request_validator.is_valid(&canonical_request));
    let canonical_bytes =
        serde_json::to_vec(&canonical_request).expect("canonical expert request serializes");
    assert!(parse_bridge_request(&canonical_bytes, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());

    // The supported projection is inside the union: schema accepted, parser accepted.
    let request = projected_expert_request();
    assert!(observation_validator.is_valid(&request["observation"]));
    assert!(projection_validator.is_valid(&request["observation"]));
    assert!(
        request_validator.is_valid(&request),
        "schema rejected the supported harness projection"
    );
    let bytes = serde_json::to_vec(&request).expect("projected request serializes");
    let parsed = parse_bridge_request(&bytes, EXO_MAX_STANDARD_REQUEST_BYTES)
        .expect("parser rejected the supported harness projection");

    let envelope = encode_bridge_request(
        "request-projection",
        "turn-projection",
        &parsed,
        EXO_MAX_STANDARD_REQUEST_BYTES,
    )
    .expect("projected request envelope encodes");
    let envelope_value: Value =
        serde_json::from_slice(&envelope).expect("projected envelope is JSON");
    assert!(envelope_validator.is_valid(&envelope_value));
    assert!(
        parse_bridge_request_envelope(&envelope, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok(),
        "parser rejected the supported projected request envelope"
    );

    // Any action outside the closed projected union is rejected by the schema and the parser, in
    // both the bare request and its envelope.
    for rejected in [
        json!({"action_id": "end:7", "action": {"kind": "end_turn"}}),
        json!({
            "action_id": "play:7:card:1:enemy:1",
            "action": {"kind": "play_card", "card_id": "card:1", "target_id": "enemy:1"}
        }),
        json!({
            "action_id": "select:7:card:1",
            "action": {"kind": "select_card", "selection_id": "sel:1", "card_id": "card:1"}
        }),
        json!({
            "action_id": "confirm:7",
            "action": {"kind": "confirm_selection", "selection_id": "sel:1"}
        }),
    ] {
        let action_id = rejected["action_id"]
            .as_str()
            .expect("rejected action carries an id");
        let mut invalid = projected_expert_request();
        invalid["observation"]["legal_actions"] = json!([rejected]);
        invalid["legal_action_ids"] = json!([action_id]);
        assert!(
            !projection_validator.is_valid(&invalid["observation"]),
            "projected observation schema accepted the out-of-union action {action_id}"
        );
        assert!(
            !request_validator.is_valid(&invalid),
            "request schema accepted the out-of-union action {action_id}"
        );
        let invalid_bytes = serde_json::to_vec(&invalid).expect("invalid projection serializes");
        assert_eq!(
            parse_bridge_request(&invalid_bytes, EXO_MAX_STANDARD_REQUEST_BYTES),
            Err(ExoWireError::InvalidRequest),
            "parser accepted the out-of-union action {action_id}"
        );
        let invalid_envelope = json!({
            "wire_version": "sts2.exo-bridge-wire-v1",
            "request_id": "request-projection",
            "turn_id": "turn-projection",
            "request": invalid
        });
        assert!(!envelope_validator.is_valid(&invalid_envelope));
        let invalid_envelope_bytes =
            serde_json::to_vec(&invalid_envelope).expect("invalid envelope serializes");
        assert_eq!(
            parse_bridge_request_envelope(&invalid_envelope_bytes, EXO_MAX_STANDARD_REQUEST_BYTES),
            Err(ExoWireError::InvalidRequest),
            "parser accepted the out-of-union action {action_id} envelope"
        );
    }

    // A marker that is not the pinned projection value is rejected by the schema and the parser.
    let mut wrong_marker = projected_expert_request();
    wrong_marker["observation"]["harness_projection"] = json!("sts2-harness/other-projection");
    assert!(!request_validator.is_valid(&wrong_marker));
    let wrong_marker_bytes =
        serde_json::to_vec(&wrong_marker).expect("wrong marker request serializes");
    assert_eq!(
        parse_bridge_request(&wrong_marker_bytes, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
    );
}
