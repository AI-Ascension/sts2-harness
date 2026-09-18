// SPDX-License-Identifier: MIT

//! Gate, catalog, and refusal fixtures for the System One decision mapper.

use super::*;

/// The options the request presented.
fn options() -> Vec<String> {
    vec![
        "play:card-17".to_owned(),
        "play:card-18".to_owned(),
        "combat.end-turn".to_owned(),
    ]
}

/// Builds a provider response carrying one choice answer.
fn response(choice: &str, confidence: f64) -> Value {
    json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {
                "type": "choice",
                "choice": choice,
                "probabilities": {
                    "play:card-17": 0.62,
                    "play:card-18": 0.21,
                    "combat.end-turn": 0.17,
                },
                "confidence": confidence,
            },
        },
        "usage": {"input_tokens": 1200, "output_tokens": 0},
    })
}

/// Maps with the default gate, yielding `Value::Null` on refusal so assertions fail loudly.
fn mapped(choice: &str, confidence: f64) -> Value {
    map_decision(
        &response(choice, confidence),
        "action",
        &options(),
        DEFAULT_CONFIDENCE_GATE,
    )
    .ok()
    .unwrap_or_default()
}

#[test]
fn a_confident_in_catalog_choice_becomes_an_action() {
    let decision = mapped("play:card-17", 0.81);
    assert_eq!(decision["decision"], json!("action"));
    assert_eq!(decision["action_id"], json!("play:card-17"));
    assert_eq!(decision["confidence"], json!(81));
    let rationale = decision["rationale"].as_str().unwrap_or_default();
    assert!(rationale.starts_with("bridge-authored evidence:"));
    assert!(rationale.contains("chose play:card-17 at p=0.62"));
    assert!(rationale.contains("runner-up play:card-18 at p=0.21"));
    assert!(rationale.contains("confidence 0.81"));
}

#[test]
fn an_unconfident_choice_asks_to_re_observe_rather_than_guessing() {
    let decision = mapped("play:card-17", 0.32);
    assert_eq!(decision["decision"], json!("reobserve"));
    assert_eq!(decision.get("action_id"), None);
    assert_eq!(decision.get("confidence"), None);
    assert!(
        decision["rationale"]
            .as_str()
            .unwrap_or_default()
            .contains("confidence 0.32")
    );
}

#[test]
fn the_gate_is_inclusive_at_its_boundary() {
    let at_gate = map_decision(
        &response("play:card-17", DEFAULT_CONFIDENCE_GATE),
        "action",
        &options(),
        DEFAULT_CONFIDENCE_GATE,
    )
    .ok()
    .unwrap_or_default();
    assert_eq!(at_gate["decision"], json!("action"));

    let below = map_decision(
        &response("play:card-17", DEFAULT_CONFIDENCE_GATE - 0.01),
        "action",
        &options(),
        DEFAULT_CONFIDENCE_GATE,
    )
    .ok()
    .unwrap_or_default();
    assert_eq!(below["decision"], json!("reobserve"));
}

#[test]
fn a_caller_can_demand_more_certainty_for_an_irreversible_action() {
    let strict = map_decision(&response("play:card-17", 0.81), "action", &options(), 0.95)
        .ok()
        .unwrap_or_default();
    assert_eq!(strict["decision"], json!("reobserve"));
}

#[test]
fn a_choice_outside_the_presented_options_is_refused() {
    assert_eq!(
        map_decision(
            &response("play:card-99", 0.99),
            "action",
            &options(),
            DEFAULT_CONFIDENCE_GATE
        )
        .err(),
        Some(DecisionError::OutOfCatalog)
    );
}

#[test]
fn an_absent_malformed_or_mistyped_answer_is_refused() {
    let cases = [
        (json!({"answers": {}}), DecisionError::MissingAnswer),
        (json!({}), DecisionError::MissingAnswer),
        (
            json!({"answers": {"action": {"type": "noul", "noul": 0.9}}}),
            DecisionError::WrongAnswerType,
        ),
        (
            json!({"answers": {"action": {"type": "choice", "confidence": 0.9}}}),
            DecisionError::MissingChoice,
        ),
        (
            json!({"answers": {"action": {"type": "choice", "choice": "play:card-17"}}}),
            DecisionError::MissingConfidence,
        ),
        (
            json!({"answers": {"action": {
                "type": "choice", "choice": "play:card-17", "confidence": 1.4}}}),
            DecisionError::MissingConfidence,
        ),
        (
            json!({"answers": {"action": {
                "type": "choice", "choice": "play:card-17", "confidence": "high"}}}),
            DecisionError::MissingConfidence,
        ),
    ];
    for (response, expected) in cases {
        assert_eq!(
            map_decision(&response, "action", &options(), DEFAULT_CONFIDENCE_GATE).err(),
            Some(expected),
            "unexpected result for {response}"
        );
    }
}

#[test]
fn an_answer_without_probabilities_still_produces_a_bounded_rationale() {
    let response = json!({"answers": {"action": {
        "type": "choice", "choice": "combat.end-turn", "confidence": 0.9}}});
    let decision = map_decision(&response, "action", &options(), DEFAULT_CONFIDENCE_GATE)
        .ok()
        .unwrap_or_default();
    assert_eq!(decision["action_id"], json!("combat.end-turn"));
    let rationale = decision["rationale"].as_str().unwrap_or_default();
    assert_eq!(
        rationale,
        "bridge-authored evidence: chose combat.end-turn, confidence 0.90"
    );
}

#[test]
fn long_identifiers_shrink_the_rationale_instead_of_breaking_its_bound() {
    let chosen = "play:".to_owned() + &"c".repeat(235);
    let other = "play:".to_owned() + &"d".repeat(235);
    let options = vec![chosen.clone(), other.clone()];
    let mut probabilities = serde_json::Map::new();
    probabilities.insert(chosen.clone(), json!(0.7));
    probabilities.insert(other, json!(0.3));
    let response = json!({"answers": {"action": {
        "type": "choice",
        "choice": chosen,
        "probabilities": Value::Object(probabilities),
        "confidence": 0.7,
    }}});
    let decision = map_decision(&response, "action", &options, DEFAULT_CONFIDENCE_GATE)
        .ok()
        .unwrap_or_default();
    let rationale = decision["rationale"].as_str().unwrap_or_default();
    assert!(!rationale.is_empty());
    assert!(rationale.len() <= MAX_RATIONALE_BYTES);
    assert!(rationale.bytes().all(|byte| (0x20..=0x7e).contains(&byte)));
    // Both identifiers together would exceed the bound, so the runner-up is the part dropped.
    assert!(!rationale.contains("runner-up"));
    assert!(rationale.contains("confidence 0.70"));
}

#[test]
fn confidence_is_carried_as_a_bounded_percentage() {
    assert_eq!(percent(0.0), 0);
    assert_eq!(percent(1.0), 100);
    assert_eq!(percent(0.555), 56);
    assert_eq!(percent(0.554), 55);
}
