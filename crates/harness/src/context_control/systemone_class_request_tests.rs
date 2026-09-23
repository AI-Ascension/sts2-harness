// SPDX-License-Identifier: MIT

//! Shape and refusal fixtures for the two-stage kind-question builder.

use serde_json::json;

use super::*;

#[test]
fn the_class_question_carries_exactly_the_kinds_as_its_options() {
    let kinds = vec!["play_card".to_owned(), "end_turn".to_owned()];
    let request = build_class_system_one_request(
        "jev-latest",
        r#"{"state_id":"combat-1"}"#,
        &kinds,
        "survive",
        &["never end the turn with unspent lethal".to_owned()],
    )
    .ok()
    .unwrap_or_default();

    // Exactly one question, named for the kind, whose options are the kinds themselves.
    let names: Vec<String> = request["questions"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(names, vec![KIND_QUESTION.to_owned()]);
    assert_eq!(request["questions"][KIND_QUESTION]["type"], json!("choice"));
    let keys: Vec<String> = request["questions"][KIND_QUESTION]["criteria"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(keys, vec!["end_turn".to_owned(), "play_card".to_owned()]);
    assert_eq!(
        request["questions"][KIND_QUESTION]["criteria"]["play_card"],
        json!("play_card")
    );

    // The kind question is framed as a kind choice, not an action choice, and carries the objective.
    let instructions = request["questions"][KIND_QUESTION]["instructions"]
        .as_str()
        .unwrap_or_default();
    assert!(instructions.contains("kind of action"), "{instructions}");
    assert!(instructions.contains("Objective: survive"));

    // The class request refuses the same inputs the action request refuses.
    assert_eq!(
        build_class_system_one_request("jev-latest", "state", &[], "", &[]).err(),
        Some(SystemOneRequestError::EmptyOptions)
    );
}
