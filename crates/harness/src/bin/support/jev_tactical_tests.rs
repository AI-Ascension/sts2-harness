// SPDX-License-Identifier: MIT

use super::*;
use crate::tactical_fixture as fixture;

fn prepared() -> Value {
    let result = prepare(fixture::body(2), 2).expect("prepared request");
    assert!(result.applied);
    result.body
}

#[test]
fn questions_cover_every_candidate_and_dimension() {
    let body = prepared();
    let questions = body["questions"].as_object().expect("questions");
    assert_eq!(questions.len(), 15);
    assert_eq!(
        questions["action"]["criteria"],
        fixture::body(2)["questions"]["action"]["criteria"]
    );
    for index in 0..2 {
        for (axis, _, _) in AXES {
            assert_eq!(questions[&key(index, axis)]["type"], "score");
        }
        assert_eq!(questions[&key(index, "evidence")]["type"], "noul");
    }
}

#[test]
fn matrix_can_select_a_different_action_from_the_baseline() {
    let body = prepared();
    let result = evaluate(&body, &fixture::reply(&body, "action-01"), 0.2).expect("assessment");
    assert_eq!(result["baseline_choice"], "action-00");
    assert_eq!(result["decision"]["action_id"], "action-01");
    assert_eq!(result["rows"][0]["action_id"], "action-01");
    assert_eq!(result["response_model"], "jev-1.13.0");
}

#[test]
fn missing_or_extra_answers_are_not_partial_successes() {
    let body = prepared();
    let mut response = fixture::reply(&body, "action-01");
    response["answers"]
        .as_object_mut()
        .expect("answers")
        .remove("tactical_0_setup");
    assert!(evaluate(&body, &response, 0.2).is_err());
    let mut response = fixture::reply(&body, "action-01");
    response["answers"]["unrequested"] = json!({"type": "noul", "noul": 1.0});
    assert!(evaluate(&body, &response, 0.2).is_err());
}

#[test]
fn malformed_score_values_and_distributions_are_refused() {
    let body = prepared();
    for (field, value) in [
        ("type", json!("choice")),
        ("score", json!(-1)),
        ("score", json!(3)),
        ("score", json!(0.3)),
        ("score", Value::Null),
        ("confidence", json!(1.1)),
        ("probabilities", json!({"0": 0.2, "1": 0.2, "2": 0.2})),
        ("probabilities", json!({"0": 1.1, "1": -0.1, "2": 0})),
        ("probabilities", json!({"0": 0.9, "1": 0, "2": 0.1, "3": 0})),
        ("legend", json!({"0": "wrong", "1": "wrong", "2": "wrong"})),
    ] {
        let mut response = fixture::reply(&body, "action-01");
        response["answers"]["tactical_0_immediate"][field] = value;
        assert!(evaluate(&body, &response, 0.2).is_err(), "field {field}");
    }
}

#[test]
fn choice_must_match_the_probability_argmax_and_catalog() {
    let body = prepared();
    for choice in ["action-01", "absent"] {
        let mut response = fixture::reply(&body, "action-01");
        response["answers"]["action"]["choice"] = json!(choice);
        assert!(evaluate(&body, &response, 0.2).is_err());
    }
}

#[test]
fn missing_evidence_never_exposes_a_candidate_that_can_be_forced() {
    let body = prepared();
    let mut response = fixture::reply(&body, "action-01");
    response["answers"]["tactical_0_evidence"]["noul"] = json!(0.1);
    let result = evaluate(&body, &response, 0.2).expect("refusal");
    assert_eq!(result["decision"]["decision"], "reobserve");
    assert!(result["decision"].get("action_id").is_none());
    assert!(result["decision"].get("candidate_action_id").is_none());
}

#[test]
fn invalid_probability_or_gate_fails_closed() {
    let body = prepared();
    for value in [json!(-0.1), json!(1.1), json!("0.9"), Value::Null] {
        let mut response = fixture::reply(&body, "action-01");
        response["answers"]["tactical_0_evidence"]["noul"] = value;
        assert!(evaluate(&body, &response, 0.2).is_err());
    }
    let response = fixture::reply(&body, "action-01");
    for gate in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(evaluate(&body, &response, gate).is_err());
    }
}

#[test]
fn a_confidence_gate_and_a_tie_both_refuse_without_guessing() {
    let body = prepared();
    let response = fixture::reply(&body, "action-01");
    assert_eq!(
        evaluate(&body, &response, 0.95).expect("gate")["decision"]["decision"],
        "reobserve"
    );
    let mut tie = response;
    for (axis, _, _) in AXES {
        tie["answers"][key(0, axis)] = tie["answers"][key(1, axis)].clone();
    }
    let result = evaluate(&body, &tie, 0.2).expect("tie");
    assert_eq!(result["decision"]["decision"], "reobserve");
    assert!(result["decision"].get("candidate_action_id").is_none());
}

#[test]
fn bounds_fall_back_to_the_original_body_without_truncation() {
    for (body, count, reason) in [
        (
            fixture::body(25),
            25,
            "candidate_bound_or_incomplete_catalog",
        ),
        (fixture::body(2), 3, "candidate_bound_or_incomplete_catalog"),
    ] {
        let result = prepare(body.clone(), count).expect("bounded fallback");
        assert!(!result.applied);
        assert_eq!(result.body, body);
        assert_eq!(result.fallback_reason, Some(reason));
    }
    let mut large = fixture::body(2);
    large["state"] = json!("x".repeat(MAX_STATE_QUESTION_BYTES));
    let result = prepare(large.clone(), 2).expect("state budget");
    assert!(!result.applied);
    assert_eq!(result.body, large);
    assert_eq!(result.fallback_reason, Some("question_batch_budget"));
}

#[test]
fn the_full_batch_not_just_the_longest_question_is_bounded() {
    let mut body = fixture::body(24);
    for value in body["questions"]["action"]["criteria"]
        .as_object_mut()
        .expect("criteria")
        .values_mut()
    {
        *value = json!("description ".repeat(100));
    }
    let result = prepare(body.clone(), 24).expect("whole batch budget");
    assert!(!result.applied);
    assert_eq!(result.body, body);
}

#[test]
fn catalog_validation_rejects_duplicates_and_control_characters() {
    for ids in [vec![], vec![""], vec!["same", "same"], vec!["newline\n"]] {
        assert!(validate_catalog(&ids.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
    }
    assert!(validate_catalog(&[String::from("action-00")]).is_ok());
}

#[test]
fn observation_entries_cannot_add_actions_to_the_catalog() {
    let observation =
        json!({"legal_actions": [{"action_id": "outside", "action": {"kind": "end_turn"}}]});
    let options = catalog_options(&observation, &[String::from("allowed")]);
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].id, "allowed");
}
