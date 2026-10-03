// SPDX-License-Identifier: MIT

//! End-to-end fixtures for asking the reward and its card in one call.
//!
//! Everything here drives the deterministic fake exchange, so no socket, credential or reachable
//! provider is involved. These assert the shape of the ask and the refusal behaviour, not any claim
//! about a real run: ADR 0053 records that no run in this repository has called the endpoint.

use super::*;
use serde_json::Map;

/// A rewards screen where the host disclosed what the card reward holds.
fn reward_request() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model_execution_id": "model-execution-9",
        "objective": "grow the deck",
        "hard_constraints": [],
        "legal_action_ids": ["choose_reward:54:reward:5", "skip_reward:54"],
        "observation": {
            "state_id": "reward-54",
            "generation": 7,
            "player": {"hp": 44, "max_hp": 80, "energy": 3, "gold": 120, "hand": []},
            "state": {
                "state": "reward",
                "choices": [
                    {"choice_id": "reward:5:CardReward", "name": "Card Reward",
                     "contents": [
                         {"choice_id": "card:21:Setup-Strike", "name": "Setup Strike",
                          "cost": 1, "description": "Gain 2 energy."},
                         {"choice_id": "card:23:Blood-Wall", "name": "Blood Wall",
                          "cost": 2, "upgraded": true}
                     ]},
                    {"choice_id": "reward:6:Gold", "name": "120 Gold"}
                ]
            }
        }
    }))
    .unwrap_or_default()
}

/// A response answering both questions.
fn both_answers(action: &str, card: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {"type": "choice", "choice": action, "confidence": 0.8,
                       "probabilities": {
                           "choose_reward:54:reward:5": 0.8,
                           "skip_reward:54": 0.2}},
            "card_choice": {"type": "choice", "choice": card, "confidence": 0.7,
                            "probabilities": {
                                "card:21:Setup-Strike": 0.7,
                                "card:23:Blood-Wall": 0.3}}
        }
    }))
    .unwrap_or_default()
}

/// Records against a fake transport, capturing every request body it was given.
fn record_capturing(request: &[u8], reply: Vec<u8>) -> (Result<Value, String>, Vec<Vec<u8>>) {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let outcome = record(
        request,
        "jev-latest",
        decision::DEFAULT_CONFIDENCE_GATE,
        &mut |body| {
            seen.push(body.to_vec());
            Ok(reply.clone())
        },
    )
    .map_err(|error| error.to_string());
    (outcome, seen)
}

/// The number of questions the transport was asked.
fn asked(sent: &[Vec<u8>]) -> usize {
    sent.first()
        .map(|body| {
            serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|body| body["questions"].as_object().map(Map::len))
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

#[test]
fn the_reward_and_its_card_are_asked_in_one_call() {
    let (record, sent) = record_capturing(
        &reward_request(),
        both_answers("choose_reward:54:reward:5", "card:21:Setup-Strike"),
    );
    assert_eq!(sent.len(), 1, "two decisions must cost one exchange");
    assert_eq!(asked(&sent), 2, "one call carries both questions");
    let sent: Value = serde_json::from_slice(&sent[0]).unwrap_or_default();
    assert!(sent["questions"]["action"].is_object());
    assert!(sent["questions"]["card_choice"].is_object());

    // The action answer is still the decision. The card answer qualifies it; it does not replace it.
    let record = record.unwrap_or_default();
    assert_eq!(record["decision"]["decision"], json!("action"));
    assert_eq!(
        record["decision"]["action_id"],
        json!("choose_reward:54:reward:5")
    );
}

#[test]
fn the_card_answer_is_recorded_beside_the_action_it_qualifies() {
    let (record, _) = record_capturing(
        &reward_request(),
        both_answers("choose_reward:54:reward:5", "card:21:Setup-Strike"),
    );
    let record = record.unwrap_or_default();
    assert_eq!(
        record["card_choice"]["reward_id"],
        json!("reward:5:CardReward")
    );
    assert_eq!(
        record["card_choice"]["advisory"],
        json!("card:21:Setup-Strike"),
        "the chosen card must be read, not merely asked for"
    );
}

#[test]
fn a_card_the_provider_never_offered_is_refused_not_recorded() {
    // The answer is outside the disclosed set. Recording it would put an identity in a run record
    // that no host object has, and letting the action stand would leave a run that believes it
    // ranked the cards and records no ranking. The whole exchange is refused instead: the question
    // was asked, so its answer is required.
    let (record, _) = record_capturing(
        &reward_request(),
        both_answers("choose_reward:54:reward:5", "card:99:Not-Offered"),
    );
    assert!(
        record.is_err(),
        "an answer outside the presented set must refuse the exchange, got {record:?}"
    );
}

#[test]
fn a_missing_card_answer_refuses_the_exchange_rather_than_recording_nothing() {
    // The provider answered only the action question. This bridge asked both, so a one-sided answer
    // is a stale or partial response, not a recordable state. Dropping it to `null` would spend the
    // call and buy no ranking while the record still claimed the two questions were settled.
    let only_action = serde_json::to_vec(&json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {"type": "choice", "choice": "choose_reward:54:reward:5",
                       "confidence": 0.8}
        }
    }))
    .unwrap_or_default();
    let (record, _) = record_capturing(&reward_request(), only_action);
    let error = record.expect_err("a missing card answer must refuse the exchange");
    assert!(
        error.contains("missing answer"),
        "the refusal must say the answer was absent, got {error}"
    );
}

#[test]
fn a_card_answer_of_the_wrong_type_refuses_the_exchange() {
    let mistyped = serde_json::to_vec(&json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {"type": "choice", "choice": "choose_reward:54:reward:5",
                       "confidence": 0.8},
            "card_choice": {"type": "boolean", "value": true, "confidence": 0.7}
        }
    }))
    .unwrap_or_default();
    let (record, _) = record_capturing(&reward_request(), mistyped);
    let error = record.expect_err("a mistyped card answer must refuse the exchange");
    assert!(
        error.contains("unexpected answer type"),
        "the refusal must name the wrong type, got {error}"
    );
}

#[test]
fn a_card_answer_without_a_choice_refuses_the_exchange() {
    let choiceless = serde_json::to_vec(&json!({
        "model": "jev-1.13.0",
        "answers": {
            "action": {"type": "choice", "choice": "choose_reward:54:reward:5",
                       "confidence": 0.8},
            "card_choice": {"type": "choice", "confidence": 0.7}
        }
    }))
    .unwrap_or_default();
    let (record, _) = record_capturing(&reward_request(), choiceless);
    let error = record.expect_err("a card answer with no choice must refuse the exchange");
    assert!(
        error.contains("missing choice"),
        "the refusal must say the choice was absent, got {error}"
    );
}

#[test]
fn a_refused_card_answer_never_becomes_a_second_action() {
    // The containment check that refuses the card answer is the same one the action answer gets, so
    // an out-of-set card can never be recorded, and never becomes the decision. This is the guard
    // against a future change that resolves the card answer against the *action* option set instead
    // of the disclosed card set, which would make `card:...` look like a dispatchable action.
    let (record, _) = record_capturing(
        &reward_request(),
        both_answers("choose_reward:54:reward:5", "choose_reward:54:reward:5"),
    );
    let error = record.expect_err("an action id is not a disclosed card and must be refused");
    assert!(
        error.contains("outside the presented options"),
        "the refusal must name the containment check, got {error}"
    );
}

#[test]
fn an_undisclosed_reward_asks_the_one_question_it_always_asked() {
    // The blind case is the one this work does not claim to have solved: with nothing disclosed
    // there is no option set to offer, so the call stays as it was rather than padding it with a
    // question whose answer could not be ranked.
    let request = serde_json::to_vec(&json!({
        "objective": "grow the deck",
        "legal_action_ids": ["choose_reward:55:reward:5", "skip_reward:55"],
        "observation": {
            "state_id": "reward-55",
            "player": {"hp": 44, "max_hp": 80, "energy": 3, "gold": 0, "hand": []},
            "state": {"state": "reward",
                      "choices": [{"choice_id": "reward:5:CardReward", "name": "Card Reward"}]}
        }
    }))
    .unwrap_or_default();
    let reply = serde_json::to_vec(&json!({
        "answers": {"action": {"type": "choice", "choice": "choose_reward:55:reward:5",
                               "confidence": 0.8}}
    }))
    .unwrap_or_default();
    let (record, sent) = record_capturing(&request, reply);
    assert_eq!(
        asked(&sent),
        1,
        "nothing disclosed means nothing to ask about"
    );
    let record = record.unwrap_or_default();
    assert!(
        record["card_choice"].is_null(),
        "a question that was never asked has no answer to record"
    );
}

#[test]
fn the_digest_covers_the_second_question_when_one_is_asked() {
    let (_, sent) = record_capturing(
        &reward_request(),
        both_answers("choose_reward:54:reward:5", "card:21:Setup-Strike"),
    );
    let sent: Value = serde_json::from_slice(&sent[0]).unwrap_or_default();
    let digest = sts2_harness::system_one_questions_digest(&sent);
    // Removing the second question must change what the digest says was asked, or a record would
    // describe two different calls as the same one.
    let mut fewer = sent.clone();
    fewer["questions"]
        .as_object_mut()
        .map(|questions| questions.remove("card_choice"));
    assert_ne!(sts2_harness::system_one_questions_digest(&fewer), digest);
}
