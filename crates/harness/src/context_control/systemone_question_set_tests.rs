// SPDX-License-Identifier: MIT

//! Pairing, bound and refusal fixtures for a multi-question System One request.

use super::*;

use crate::context_control::systemone_request::{
    ACTION_QUESTION, MAX_DESCRIPTION_BYTES, MAX_OPTIONS,
};
use serde_json::{Map, json};

fn option(id: &str) -> SystemOneOption {
    SystemOneOption {
        id: id.to_owned(),
        description: String::new(),
    }
}

/// Two described option sets, the shape a reward screen produces.
fn actions() -> Vec<SystemOneOption> {
    vec![option("choose_reward:54:reward:5"), option("proceed:54")]
}

fn cards() -> Vec<SystemOneOption> {
    vec![option("card:21:Setup-Strike"), option("card:23:Blood-Wall")]
}

/// Builds the two-question request, or yields `Value::Null` on refusal.
fn built(questions: &[SystemOneQuestion<'_>]) -> Value {
    build_choice_questions_request("jev-latest", r#"{"state_id":"reward-54"}"#, questions)
        .ok()
        .unwrap_or_default()
}

#[test]
fn carries_both_questions_in_one_request() {
    let request = built(&[
        SystemOneQuestion::new(ACTION_QUESTION, &actions(), "choose the action"),
        SystemOneQuestion::new("card_choice", &cards(), "choose the card"),
    ]);
    let questions = request["questions"].as_object().map_or(0, Map::len);
    assert_eq!(questions, 2, "both questions must be carried in one call");
    assert_eq!(request["questions"]["action"]["type"], json!("choice"));
    assert_eq!(request["questions"]["card_choice"]["type"], json!("choice"));
    assert_eq!(
        request["questions"]["action"]["instructions"],
        json!("choose the action")
    );
    assert_eq!(
        request["questions"]["card_choice"]["instructions"],
        json!("choose the card")
    );
}

#[test]
fn each_question_keeps_its_own_option_set() {
    let request = built(&[
        SystemOneQuestion::new(ACTION_QUESTION, &actions(), "a"),
        SystemOneQuestion::new("card_choice", &cards(), "b"),
    ]);
    let action = &request["questions"]["action"]["criteria"];
    let card = &request["questions"]["card_choice"]["criteria"];
    assert!(action.get("choose_reward:54:reward:5").is_some());
    assert!(
        card.get("choose_reward:54:reward:5").is_none(),
        "a card set must not carry an action option"
    );
    assert!(card.get("card:23:Blood-Wall").is_some());
}

#[test]
fn the_budget_is_the_longest_single_question_not_the_whole_set() {
    // A state sized so that it and one maximum question sit exactly on the ceiling. A second
    // question of the same size would put the two together over it, so a whole-set budget would
    // refuse this request. Asking two questions does not make either of them larger, which is the
    // property this contract keeps: the ceiling is shared with the longest single question.
    let options = many_options(MAX_OPTIONS);
    let probe = build_choice_questions_request(
        "jev-latest",
        "s",
        &[SystemOneQuestion::new("one", &options, "i")],
    )
    .ok()
    .unwrap_or_default();
    let question_bytes = probe["questions"]["one"].to_string().len();
    let state = "y".repeat(MAX_STATE_AND_QUESTION_BYTES - question_bytes);
    let other = many_options(MAX_OPTIONS)
        .into_iter()
        .map(|mut option| {
            // Same length as the first set's ids, so the second question is byte-identical in
            // size. A longer id would make that question the longest one and move the ceiling,
            // which is the correct behaviour and would test nothing about the set as a whole.
            option.id = format!("otlr:{}", &option.id[5..]);
            option
        })
        .collect::<Vec<_>>();
    let pair = build_choice_questions_request(
        "jev-latest",
        &state,
        &[
            SystemOneQuestion::new("one", &options, "i"),
            SystemOneQuestion::new("two", &other, "i"),
        ],
    );
    assert!(
        pair.is_ok(),
        "the ceiling is the longest question, not the whole set: {:?}",
        pair.as_ref().err()
    );
}

#[test]
fn an_oversized_question_is_still_refused_on_its_own() {
    // One byte more of state than the ceiling allows beside this question. The refusal is on the
    // budget term, and the builder still does not truncate the state to make it fit.
    let huge = many_options(MAX_OPTIONS);
    let probe = build_choice_questions_request(
        "jev-latest",
        "s",
        &[SystemOneQuestion::new("one", &huge, "i")],
    )
    .ok()
    .unwrap_or_default();
    let question_bytes = probe["questions"]["one"].to_string().len();
    let state = "y".repeat(MAX_STATE_AND_QUESTION_BYTES - question_bytes + 1);
    let built = build_choice_questions_request(
        "jev-latest",
        &state,
        &[SystemOneQuestion::new("one", &huge, "i")],
    );
    assert_eq!(
        built.err(),
        Some(SystemOneRequestError::OverBudget),
        "a state one byte over the ceiling is refused, not truncated"
    );
}

/// `count` distinct options whose descriptions are at the per-option bound.
fn many_options(count: usize) -> Vec<SystemOneOption> {
    (0..count)
        .map(|index| SystemOneOption {
            id: format!("option:{index}"),
            description: "d".repeat(MAX_DESCRIPTION_BYTES),
        })
        .collect()
}

#[test]
fn two_questions_under_one_name_are_refused() {
    let built = built(&[
        SystemOneQuestion::new("same", &actions(), "a"),
        SystemOneQuestion::new("same", &cards(), "b"),
    ]);
    assert!(
        built.is_null(),
        "one name cannot carry two questions: the second would be unanswerable"
    );
}

#[test]
fn refuses_an_unnamed_oversized_set_and_an_empty_one() {
    let nameless = build_choice_questions_request(
        "jev-latest",
        r#"{"s":1}"#,
        &[SystemOneQuestion::new("", &actions(), "i")],
    );
    assert_eq!(nameless.err(), Some(SystemOneRequestError::InvalidQuestion));

    let empty = build_choice_questions_request("jev-latest", r#"{"s":1}"#, &[]);
    assert_eq!(empty.err(), Some(SystemOneRequestError::EmptyOptions));
}

#[test]
fn refuses_more_questions_than_the_bound_admits() {
    let options = [option("only")];
    let names: Vec<String> = (0..=MAX_QUESTIONS)
        .map(|index| format!("q{index}"))
        .collect();
    let mut questions = Vec::new();
    for name in &names {
        questions.push(SystemOneQuestion::new(name, &options, "i"));
    }
    let built = build_choice_questions_request("jev-latest", r#"{"s":1}"#, &questions);
    assert_eq!(
        built.err(),
        Some(SystemOneRequestError::TooManyQuestions),
        "the question count is bounded so a caller cannot spend without limit"
    );
}

#[test]
fn an_option_set_is_validated_for_every_question_not_only_the_first() {
    let empty: [SystemOneOption; 0] = [];
    let built = build_choice_questions_request(
        "jev-latest",
        r#"{"s":1}"#,
        &[
            SystemOneQuestion::new("action", &actions(), "i"),
            SystemOneQuestion::new("card_choice", &empty, "i"),
        ],
    );
    assert_eq!(
        built.err(),
        Some(SystemOneRequestError::EmptyOptions),
        "a question with no options must not be sent: its answer could not be ranked"
    );
}

#[test]
fn the_single_question_builder_is_this_builder_with_one_question() {
    let single = built(&[SystemOneQuestion::new(ACTION_QUESTION, &actions(), "i")]);
    let direct = super::super::systemone_request::build_described_system_one_request(
        "jev-latest",
        r#"{"state_id":"reward-54"}"#,
        &actions(),
        "",
        &[],
    )
    .ok()
    .unwrap_or_default();
    // The instruction text differs because the action builder composes its own from the objective,
    // so the comparison is over the shape both must agree on: one question, the same criteria, the
    // same state and model. The digest is checked separately over the questions both produce.
    assert_eq!(
        single["questions"][ACTION_QUESTION]["criteria"],
        direct["questions"][ACTION_QUESTION]["criteria"]
    );
    assert_eq!(single["state"], direct["state"]);
    assert_eq!(single["model"], direct["model"]);
}

#[test]
fn the_digest_covers_every_question_not_only_the_action() {
    // The digest is the identity of what was asked. If adding a question left it unchanged, a
    // record carrying it would describe two different calls as the same one.
    let mut pair = built(&[
        SystemOneQuestion::new(ACTION_QUESTION, &actions(), "i"),
        SystemOneQuestion::new("card_choice", &cards(), "i"),
    ]);
    let both = crate::context_control::system_one_questions_digest(&pair);
    pair["questions"]
        .as_object_mut()
        .map(|questions| questions.remove("card_choice"));
    let one = crate::context_control::system_one_questions_digest(&pair);
    assert_ne!(
        one, both,
        "the digest must change when what was asked changes"
    );
}
