// SPDX-License-Identifier: MIT

//! Selection-mode fixtures for the System One bridge.
//!
//! These exercise how one record states which selection path produced its decision: a single
//! question, two stages when the presented set is above the bound, or a forced action taken
//! without asking. Every exchange is a deterministic fake, so nothing here opens a socket.

use super::*;

#[test]
fn a_forced_action_records_that_no_provider_call_happened() {
    let record = record(
        &forced_turn_request(),
        "jev-latest",
        decision::DEFAULT_CONFIDENCE_GATE,
        &mut |_| Err("the bridge must not ask".into()),
    )
    .expect("record");
    assert_eq!(record["provider_call"], json!(false));
    assert_eq!(record["provider_request"], json!(null));
    assert_eq!(record["provider_response"], json!(null));
    assert_eq!(record["decision"]["action_id"], json!("end:13"));
}

/// The criteria keys of the single question a request body carries.
fn question_ids(body: &Value, question: &str) -> Vec<String> {
    body["questions"][question]["criteria"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

/// The question name a body carries, which is `kind` or `action` for the two stages.
fn question_name(body: &Value) -> &'static str {
    if body["questions"].get(KIND_QUESTION).is_some() {
        KIND_QUESTION
    } else {
        ACTION_QUESTION
    }
}

/// A reply to whichever stage `body` is: a kind choice, or an action choice.
fn stage_reply(body: &Value, kind_choice: &str, action_choice: &str) -> Vec<u8> {
    let question = question_name(body);
    let choice = if question == KIND_QUESTION {
        kind_choice
    } else {
        action_choice
    };
    let mut answers = serde_json::Map::new();
    answers.insert(
        question.to_owned(),
        json!({"type": "choice", "choice": choice, "confidence": 0.90}),
    );
    serde_json::to_vec(&json!({
        "model": "jev-1.13.0",
        "answers": answers,
        "usage": {"input_tokens": 10, "output_tokens": 0},
    }))
    .unwrap_or_default()
}

/// A request whose presented set is above the option bound: 25 plays and ending the turn.
fn two_stage_request() -> Vec<u8> {
    let mut ids: Vec<String> = (0..25).map(|index| format!("play:card-{index}")).collect();
    ids.push("combat.end-turn".to_owned());
    let mut legal: Vec<Value> = (0..25)
        .map(|index| {
            json!({
                "action_id": format!("play:card-{index}"),
                "action": {
                    "kind": "play_card",
                    "card_id": format!("card-{index}"),
                    "target_id": "enemy-0",
                },
            })
        })
        .collect();
    legal.push(json!({"action_id": "combat.end-turn", "action": {"kind": "end_turn"}}));
    serde_json::to_vec(&json!({
        "model_execution_id": "model-execution-7",
        "objective": "survive the turn",
        "hard_constraints": ["never end the turn with unspent lethal"],
        "legal_action_ids": ids,
        "observation": {
            "state_id": "combat-1",
            "generation": 3,
            "player": {"hp": 30, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "combat", "turn_index": 2},
            "legal_actions": legal,
        },
    }))
    .unwrap_or_default()
}

/// Runs `record` against a fake that answers the kind stage then the action stage, capturing both
/// request bodies in order.
fn two_stage_record(kind_choice: &str, action_choice: &str) -> (Result<Value, String>, Vec<Value>) {
    let mut seen: Vec<Value> = Vec::new();
    let outcome = {
        let mut ask = |body: &[u8]| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
            let parsed: Value = serde_json::from_slice(body)?;
            let reply = stage_reply(&parsed, kind_choice, action_choice);
            seen.push(parsed);
            Ok(reply)
        };
        record(
            &two_stage_request(),
            "jev-latest",
            decision::DEFAULT_CONFIDENCE_GATE,
            &mut ask,
        )
        .map_err(|error| error.to_string())
    };
    (outcome, seen)
}

#[test]
fn an_option_set_above_the_bound_is_asked_as_a_kind_then_an_action() {
    let (outcome, seen) = two_stage_record("play_card", "play:card-3");
    let record = outcome.expect("two-stage record");

    // Two stages, in order: the kind question first, the action question second.
    assert_eq!(seen.len(), 2, "expected exactly two stages");
    assert_eq!(question_name(&seen[0]), KIND_QUESTION);
    assert_eq!(question_name(&seen[1]), ACTION_QUESTION);

    // The class question offers the kinds; the target question offers only the chosen kind's options.
    assert_eq!(
        question_ids(&seen[0], KIND_QUESTION),
        vec!["end_turn".to_owned(), "play_card".to_owned()]
    );
    let target = question_ids(&seen[1], ACTION_QUESTION);
    assert_eq!(target.len(), 25, "the target set is the chosen kind");
    assert!(!target.contains(&"combat.end-turn".to_owned()));

    // The record states the mode and carries both stages; the decision is a function of the action
    // response, and the action stage is the one the published fields read.
    assert_eq!(record["selection_mode"], json!("two_stage"));
    assert_eq!(record["provider_call"], json!(true));
    assert_eq!(record["provider_request"], seen[1]);
    assert_eq!(record["class_question"]["provider_request"], seen[0]);
    assert_eq!(record["decision"]["decision"], json!("action"));
    assert_eq!(record["decision"]["action_id"], json!("play:card-3"));
}

#[test]
fn the_target_question_is_restricted_to_the_kind_the_first_stage_chose() {
    let (outcome, seen) = two_stage_record("end_turn", "combat.end-turn");
    let record = outcome.expect("two-stage record");
    assert_eq!(seen.len(), 2);
    // Choosing "end_turn" leaves the target question exactly one option, and only that option.
    assert_eq!(
        question_ids(&seen[1], ACTION_QUESTION),
        vec!["combat.end-turn".to_owned()]
    );
    assert_eq!(record["decision"]["action_id"], json!("combat.end-turn"));
}

#[test]
fn a_kind_outside_the_presented_classes_is_refused() {
    // The first stage names a kind the host never presented, so the second stage must not be asked.
    let mut asked = 0;
    let outcome = record(
        &two_stage_request(),
        "jev-latest",
        decision::DEFAULT_CONFIDENCE_GATE,
        &mut |body| {
            asked += 1;
            let parsed: Value = serde_json::from_slice(body)?;
            Ok(stage_reply(&parsed, "shop", "play:card-0"))
        },
    );
    assert!(outcome.is_err(), "an unlisted kind must fail closed");
    assert_eq!(asked, 1, "the target question must not be asked");
}

#[test]
fn a_single_question_set_records_the_single_mode() {
    let record = record(
        &request(),
        "jev-latest",
        decision::DEFAULT_CONFIDENCE_GATE,
        &mut |_| Ok(response("play:card-17", 0.81)),
    )
    .expect("record");
    assert_eq!(record["selection_mode"], json!("single"));
}

#[test]
fn a_suppressed_split_asks_one_question_and_records_single() {
    // A profile that permits only one exchange must not split, even above the bound: it asks the
    // whole set in one question and records that one question was asked.
    let mut questions: Vec<&'static str> = Vec::new();
    let record = record_profile(
        &two_stage_request(),
        "jev-latest",
        decision::DEFAULT_CONFIDENCE_GATE,
        &mut |body| {
            let parsed: Value = serde_json::from_slice(body)?;
            questions.push(question_name(&parsed));
            Ok(stage_reply(&parsed, "play_card", "play:card-3"))
        },
        false,
        false,
    )
    .expect("record");
    assert_eq!(questions, vec![ACTION_QUESTION], "exactly one question");
    assert_eq!(record["selection_mode"], json!("single"));
    assert_eq!(record["provider_call"], json!(true));
    assert!(record.get("class_question").is_none());
    assert_eq!(record["decision"]["action_id"], json!("play:card-3"));
}

#[test]
fn a_forced_action_records_the_forced_mode() {
    let record = record(
        &forced_turn_request(),
        "jev-latest",
        decision::DEFAULT_CONFIDENCE_GATE,
        &mut |_| Err("the bridge must not ask".into()),
    )
    .expect("record");
    assert_eq!(record["selection_mode"], json!("forced"));
}
