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

/// Questions whose option identifiers and descriptions are both at the bounds a caller can reach.
///
/// `MAX_OPTIONS` options, each with a 240-byte identifier and description. Identifiers are padded
/// rather than numbered so every question is the same size, which lets a test place a total exactly
/// on a ceiling.
fn maximal_questions(count: usize) -> Vec<(String, Vec<SystemOneOption>)> {
    (0..count)
        .map(|index| {
            (
                format!("q{index:0>3}"),
                (0..MAX_OPTIONS)
                    .map(|slot| SystemOneOption {
                        id: format!("{:0>240}", format!("id{index}-{slot}")),
                        description: "d".repeat(MAX_DESCRIPTION_BYTES),
                    })
                    .collect(),
            )
        })
        .collect()
}

/// The questions of a maximal set, borrowing the name and options the set owns.
fn borrowing(questions: &[(String, Vec<SystemOneOption>)]) -> Vec<SystemOneQuestion<'_>> {
    questions
        .iter()
        .map(|(name, options)| SystemOneQuestion::new(name, options, "i"))
        .collect()
}

/// Bytes a request costs before its state. Measuring with the state emptied makes the figure depend
/// only on the question set, so a test can size a state that lands a total on a chosen byte.
fn envelope_of(questions: &[SystemOneQuestion<'_>]) -> usize {
    let mut envelope =
        build_choice_questions_request("jev-latest", "s", questions).unwrap_or_default();
    envelope["state"] = json!("");
    envelope.to_string().len()
}

/// Bytes the largest question in a maximal set costs, measured on its own. Every question is the
/// same size, so measuring the first measures the longest; it is built alone because a set the total
/// bound refuses cannot be measured whole.
fn longest_of(questions: &[SystemOneQuestion<'_>]) -> usize {
    build_choice_questions_request("jev-latest", "s", &questions[..1])
        .unwrap_or_default()["questions"]["q000"]
        .to_string()
        .len()
}

/// The largest state that keeps a request inside the per-request ceiling for this question set.
fn largest_state_inside_request_budget(envelope: usize) -> usize {
    MAX_REQUEST_BYTES.saturating_sub(envelope)
}

#[test]
fn four_questions_beyond_the_per_request_ceiling_are_refused_not_truncated() {
    // Four maximum questions and the largest state the shared ceiling admits. Each question is
    // inside the ceiling the state shares, so that check passes and the total is what runs past the
    // per-request ceiling. This is the term the earlier check could not see: before it, a caller
    // could fill the set towards MAX_QUESTIONS and only the provider would have noticed. Four is the
    // smallest set that gets there — three still fit inside the per-request ceiling at the state the
    // shared ceiling allows, so there the two bounds agree.
    let owned = maximal_questions(4);
    let questions = borrowing(&owned);
    let state = "y".repeat(MAX_STATE_AND_QUESTION_BYTES - longest_of(&questions));
    let built = build_choice_questions_request("jev-latest", &state, &questions);
    assert!(
        envelope_of(&questions) + state.len() > MAX_REQUEST_BYTES,
        "this fixture must actually reach the per-request ceiling, not approximate it"
    );
    assert!(
        state.len() + longest_of(&questions) <= MAX_STATE_AND_QUESTION_BYTES,
        "the shared ceiling must not be the bound that fires here"
    );
    assert_eq!(
        built.err(),
        Some(SystemOneRequestError::RequestTooLarge),
        "a set that inflates the whole request past the per-request ceiling must be refused"
    );
}

#[test]
fn the_per_request_ceiling_is_refused_on_exactly_the_byte_past_it() {
    // The boundary, from below and above it. Four maximum questions leave 6,381 bytes of state at
    // the per-request ceiling against the 34,383 the shared ceiling allows, so the per-request bound
    // owns a band of states the shared bound would have admitted: a state of exactly 6,381 bytes
    // serializes to exactly the ceiling and is admitted, and one byte more is refused while the
    // shared ceiling is still nowhere near. Four is the smallest maximum-size set where this is
    // true — at three the per-request ceiling is still above the shared one, so there the two
    // refusals coincide and an exact-byte fixture would prove nothing.
    let owned = maximal_questions(4);
    let questions = borrowing(&owned);
    let envelope = envelope_of(&questions);
    let at_ceiling = largest_state_inside_request_budget(envelope);
    assert!(
        at_ceiling < MAX_STATE_AND_QUESTION_BYTES - longest_of(&questions),
        "this fixture needs the per-request ceiling to bite below the shared one"
    );
    let admitted =
        build_choice_questions_request("jev-latest", &"y".repeat(at_ceiling), &questions);
    assert!(
        admitted.is_ok(),
        "a request exactly on the per-request ceiling is admitted: {:?}",
        admitted.as_ref().err()
    );
    let over =
        build_choice_questions_request("jev-latest", &"y".repeat(at_ceiling + 1), &questions);
    assert_eq!(
        over.err(),
        Some(SystemOneRequestError::RequestTooLarge),
        "one byte past the per-request ceiling is refused, not truncated"
    );
}

#[test]
fn a_request_inside_both_ceilings_is_still_admitted() {
    // The admitted side of that band: a four-question request one byte below the per-request
    // ceiling. Without this, a bound that fired too eagerly would pass every other test in this
    // file.
    let owned = maximal_questions(4);
    let questions = borrowing(&owned);
    let inside = largest_state_inside_request_budget(envelope_of(&questions)) - 1;
    let admitted = build_choice_questions_request("jev-latest", &"y".repeat(inside), &questions);
    assert!(
        admitted.is_ok(),
        "a request inside both ceilings is admitted: {:?}",
        admitted.as_ref().err()
    );
}

#[test]
fn the_digest_is_unchanged_by_adding_the_total_request_bound() {
    // The digest is what a consumer pins. It is taken over the serialized `questions` object alone,
    // and this change reads the request rather than rewriting it, so every request that was admitted
    // before is byte-identical now and every digest a consumer recorded still matches. This test
    // states that as a contract instead of leaving it to be re-derived: the same question set built
    // with a state far from both ceilings digests identically to one built with a state inside them.
    let owned = maximal_questions(2);
    let questions = borrowing(&owned);
    let small = build_choice_questions_request("jev-latest", r#"{"s":1}"#, &questions)
        .ok()
        .unwrap_or_default();
    let near = build_choice_questions_request(
        "jev-latest",
        &"y".repeat(MAX_STATE_AND_QUESTION_BYTES - longest_of(&questions)),
        &questions,
    )
    .ok()
    .unwrap_or_default();
    assert_ne!(
        small.to_string(),
        near.to_string(),
        "these two states must differ, or the fixture proves nothing"
    );
    assert_eq!(
        crate::context_control::system_one_questions_digest(&small),
        crate::context_control::system_one_questions_digest(&near),
        "a total-size bound must not move any question-set digest"
    );
}

#[test]
fn a_two_question_request_far_inside_the_ceiling_is_unaffected() {
    // The live path: at most two questions, and tens of thousands of tokens of headroom against
    // the per-request ceiling. Adding the bound must not refuse a request this contract already
    // admitted, which is what makes it safe to add at all.
    let request = built(&[
        SystemOneQuestion::new(ACTION_QUESTION, &actions(), "a"),
        SystemOneQuestion::new("card_choice", &cards(), "b"),
    ]);
    assert!(
        !request.is_null(),
        "the two-question path must still be admitted"
    );
    assert!(
        request.to_string().len() <= MAX_REQUEST_BYTES,
        "the two-question path is far inside the per-request ceiling"
    );
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
