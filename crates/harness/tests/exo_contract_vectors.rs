// SPDX-License-Identifier: MIT

//! Consolidated Exo contract vectors (issue #139).
//!
//! Covers every semantic decision variant, malformed variants, illegal-action binding, wrong
//! correlation, ordinary/map request bounds, and incompatible request schemas at the public
//! boundary. Capability-unavailable and capability-schema-version vectors live in the unit tests of
//! `crates/harness/src/exo/capability_tests.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::json;
use sts2_harness::{
    Decision, DecisionError, EXO_MAP_REQUEST_OVERHEAD_BYTES, EXO_MAX_MAP_REQUEST_BYTES,
    EXO_MAX_STANDARD_REQUEST_BYTES, ExoDecisionRequest, ExoError, ModelExecutionId,
    SanitizedObservation, parse_decision,
};

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn observation(state_id: &str, generation: u64) -> SanitizedObservation {
    SanitizedObservation::new(json!({
        "state_id": state_id,
        "generation": generation,
        "player": {"hp": 50, "max_hp": 50, "energy": 3, "gold": 99,
            "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {"state": "map", "node_id": "start", "options": ["z001"]},
        "legal_actions": [{"action_id": "move-1",
            "action": {"kind": "select_map_node", "node_id": "z001"}}]
    }))
    .expect("test observation must satisfy the fair-play contract")
}

fn valid_request() -> ExoDecisionRequest {
    ExoDecisionRequest::new(
        ModelExecutionId::new(1).expect("model execution id"),
        REVISION,
        "state-1",
        1,
        observation("state-1", 1),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        8 * 1024,
    )
    .expect("test request must satisfy the provider contract")
}

fn parse(bytes: &[u8]) -> Result<Decision, DecisionError> {
    parse_decision(bytes)
}

#[test]
fn decision_vectors_cover_every_semantic_variant() {
    assert_eq!(
        parse(
            br#"{"decision":"action","action_id":"move-1","rationale":"advance","confidence":75}"#
        ),
        Ok(Decision::Action {
            action_id: String::from("move-1"),
            rationale: String::from("advance"),
            confidence: Some(75),
        })
    );
    assert!(matches!(
        parse(br#"{"decision":"plan","action_ids":["move-1"],"rationale":"plan"}"#),
        Ok(Decision::Plan { .. })
    ));
    assert!(matches!(
        parse(br#"{"decision":"wait","rationale":"hold"}"#),
        Ok(Decision::Wait { .. })
    ));
    assert!(matches!(
        parse(br#"{"decision":"reobserve","rationale":"refresh"}"#),
        Ok(Decision::Reobserve { .. })
    ));
    assert!(matches!(
        parse(br#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"recover"}"#),
        Ok(Decision::Recovery { .. })
    ));
    assert!(matches!(
        parse(br#"{"decision":"recovery","recovery_kind":"reconcile","operation_id":"op-1","rationale":"reconcile"}"#),
        Ok(Decision::Recovery { operation_id: Some(_), .. })
    ));
}

#[test]
fn decision_vectors_reject_malformed_variants() {
    let malformed: [&[u8]; 5] = [
        br#"{"decision":"action","action_id":"move-1"}"#,
        br#"{"decision":"action","action_id":"move-1","rationale":"x","extra":1}"#,
        br#"{"decision":"action","action_id":"move-1","action_id":"move-1","rationale":"x"}"#,
        br#"{"decision":"recovery","recovery_kind":"reconcile","rationale":"x"}"#,
        br#"{"decision":"unknown","rationale":"x"}"#,
    ];
    for bytes in malformed {
        assert!(parse(bytes).is_err(), "must reject {:?}", bytes);
    }
}

#[test]
fn action_binding_rejects_an_illegal_action() {
    let parsed = parse(br#"{"decision":"action","action_id":"move-9","rationale":"x"}"#)
        .expect("decision parses");
    assert_eq!(
        parsed.bind(&[String::from("move-1")]),
        Err(DecisionError::IllegalAction)
    );
}

#[test]
fn wrong_correlation_fails_before_any_transport() {
    let mismatched = ExoDecisionRequest::new(
        ModelExecutionId::new(2).expect("model execution id"),
        REVISION,
        "state-2",
        2,
        observation("state-1", 1),
        vec![String::from("move-1")],
        "objective",
        Vec::new(),
        8 * 1024,
    );
    assert_eq!(mismatched.err(), Some(ExoError::InvalidRequest));
}

#[test]
fn request_bounds_and_schema_are_enforced() {
    let request = valid_request();
    assert!(request.encode(EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
    assert_eq!(request.encode(1), Err(ExoError::RequestTooLarge));
    assert_eq!(
        EXO_MAX_MAP_REQUEST_BYTES,
        EXO_MAX_STANDARD_REQUEST_BYTES + 256 * 1024 + EXO_MAP_REQUEST_OVERHEAD_BYTES
    );

    let mut incompatible = valid_request();
    incompatible.schema = String::from("sts2.exo-decision-v9");
    assert_eq!(
        incompatible.encode(EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoError::InvalidRequest)
    );
}
