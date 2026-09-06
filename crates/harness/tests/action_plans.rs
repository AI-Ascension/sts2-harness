// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use std::{cell::Cell, collections::VecDeque, rc::Rc};
use sts2_harness::{
    ActionKind, Decision, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeStage, ExoConfig, ExoDecisionSource, ExoProvider, ExoSession,
    ExoTransport, ExoTransportError, ModelExecutionId, PolicyError, parse_decision,
};

struct Provider {
    responses: VecDeque<Value>,
    calls: Rc<Cell<usize>>,
}
impl ExoTransport for Provider {
    fn exchange(&mut self, _: &[u8], _: usize, _: u32) -> Result<Vec<u8>, ExoTransportError> {
        self.calls.set(self.calls.get() + 1);
        let value = self
            .responses
            .pop_front()
            .ok_or(ExoTransportError::Unavailable)?;
        serde_json::to_vec(&value).map_err(|_| ExoTransportError::MalformedResponse)
    }
    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

fn source(ids: &[&str]) -> (ExoDecisionSource<Provider>, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let provider = Provider {
        calls: calls.clone(),
        responses: VecDeque::from([
            json!({"decision":"plan", "action_ids":ids, "rationale":"Use the visible choices in order"}),
            json!({"decision":"reobserve", "rationale":"New information requires a decision"}),
        ]),
    };
    let config = ExoConfig::new(
        "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        65536,
        8192,
        2000,
    )
    .expect("valid config");
    (
        ExoDecisionSource::new(ExoSession::new(ExoProvider::new(provider, config))),
        calls,
    )
}

fn card(id: &str) -> Value {
    json!({"card_id":id,"name":"Visible card","cost":1,"upgraded":false})
}

fn observation(shop: bool, generation: u64) -> Value {
    let state = if shop {
        json!({"state":"shop", "items":[
            {"item_id":"a","name":"Visible A","price":10},
            {"item_id":"b","name":"Visible B","price":20}]})
    } else {
        json!({"state":"combat","turn_index":1,"enemies":[
            {"enemy_id":"enemy","name":"Visible enemy","hp":40,"max_hp":40,
             "intent":{"kind":"attack","damage":5,"hits":1}}]})
    };
    let actions: Vec<_> = ["a", "b"]
        .iter()
        .map(|id| {
            let action = if shop {
                json!({"kind":"shop_purchase","item_id":id})
            } else {
                json!({"kind":"play_card","card_id":id,"target_id":"enemy"})
            };
            json!({"action_id":format!("choice-{generation}-{id}"), "action":action})
        })
        .collect();
    json!({"state_id":format!("state-{generation}"), "generation":generation,"visible_seed":"seed",
        "player":{"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[card("a"),card("b")],
            "deck":[],"discard":[],"exhaust":[]}, "state":state, "legal_actions":actions})
}

fn successor(shop: bool) -> Value {
    let mut value = observation(shop, 2);
    value["legal_actions"]
        .as_array_mut()
        .expect("actions")
        .remove(0);
    if shop {
        value["state"]["items"]
            .as_array_mut()
            .expect("items")
            .remove(0);
        value["player"]["gold"] = json!(89);
    } else {
        value["player"]["hand"]
            .as_array_mut()
            .expect("hand")
            .remove(0);
        value["player"]["energy"] = json!(2);
        value["state"]["enemies"][0]["hp"] = json!(30);
    }
    value
}

fn input(value: Value) -> DecisionInput {
    let generation = value["generation"].as_u64().expect("generation");
    let state_id = value["state_id"].as_str().expect("state ID").to_owned();
    let shop = value["state"]["state"] == "shop";
    let actions = value["legal_actions"]
        .as_array()
        .expect("catalog")
        .iter()
        .map(|action| {
            EpisodeLegalAction::new(
                action["action_id"].as_str().expect("action ID"),
                if shop {
                    ActionKind::ShopPurchase
                } else {
                    ActionKind::PlayCard
                },
            )
            .expect("action")
        })
        .collect();
    DecisionInput::new(
        ModelExecutionId::new(generation + 10).expect("execution"),
        EpisodeObservation::new(
            &state_id,
            generation,
            if shop {
                EpisodeStage::Shop
            } else {
                EpisodeStage::Combat
            },
            true,
            false,
            true,
            value,
        )
        .expect("observation"),
        EpisodeLegalActionSet::new(&state_id, generation, actions).expect("catalog"),
        "complete the run",
        vec![],
    )
}

#[test]
fn one_provider_response_drives_two_settled_moves_with_fresh_ids_and_original_identity() {
    for shop in [false, true] {
        let (mut source, calls) = source(&["choice-1-a", "choice-1-b"]);
        let first = input(observation(shop, 1));
        assert!(
            matches!(source.decide(&first),Ok(Decision::Action { action_id, .. }) if action_id == "choice-1-a")
        );
        assert_eq!(source.model_execution_id(), Some(first.execution_id));
        let second = input(successor(shop));
        assert_eq!(source.decide(&second), Err(PolicyError::InputBlocked));
        assert_eq!(calls.get(), 1);
        source.action_completed(true);
        assert!(
            matches!(source.decide(&second),Ok(Decision::Action { action_id, .. }) if action_id == "choice-2-b")
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(source.model_execution_id(), Some(first.execution_id));
        source.action_completed(true);
        assert!(matches!(
            source.decide(&second),
            Ok(Decision::Reobserve { .. })
        ));
        assert_eq!(calls.get(), 2);
    }
}

#[test]
fn rejection_discards_the_tail_and_never_retries_its_action() {
    let (mut source, calls) = source(&["choice-1-a", "choice-1-b"]);
    source.decide(&input(observation(false, 1))).expect("first");
    source.action_completed(false);
    assert!(matches!(
        source.decide(&input(successor(false))),
        Ok(Decision::Reobserve { .. })
    ));
    assert_eq!(calls.get(), 2);
}

#[test]
fn new_cards_turns_intents_and_changed_offers_require_a_new_provider_call() {
    for case in 0..8 {
        let shop = case >= 4;
        let (mut source, calls) = source(&["choice-1-a", "choice-1-b"]);
        source.decide(&input(observation(shop, 1))).expect("first");
        source.action_completed(true);
        let mut next = successor(shop);
        match case {
            0 => next["player"]["hand"]
                .as_array_mut()
                .expect("hand")
                .push(card("new")),
            1 => next["player"]["hand"]
                .as_array_mut()
                .expect("hand")
                .push(card("a")),
            2 => next["state"]["turn_index"] = json!(2),
            3 => next["state"]["enemies"][0]["intent"]["damage"] = json!(10),
            4 => next["state"]["items"][0]["price"] = json!(21),
            5 => next["state"]["items"][0]["name"] = json!("Changed offer"),
            6 => next["legal_actions"][0]["action"]["item_id"] = json!("other"),
            _ => next["visible_seed"] = json!("another-seed"),
        }
        assert!(
            matches!(source.decide(&input(next)), Ok(Decision::Reobserve { .. })),
            "case {case}"
        );
        assert_eq!(calls.get(), 2, "case {case}");
    }
}

#[test]
fn plan_parser_rejects_empty_oversized_duplicate_and_mixed_responses() {
    for value in [
        json!({"decision":"plan","action_ids":[],"rationale":"valid rationale"}),
        json!({"decision":"plan","action_ids":["a","a"],"rationale":"valid rationale"}),
        json!({"decision":"plan","action_ids":[1],"rationale":"valid rationale"}),
        json!({"decision":"plan","action_ids":["a"],"action_id":"a","rationale":"valid rationale"}),
        json!({"decision":"action","action_ids":["a"],"action_id":"a","rationale":"valid rationale"}),
        json!({"decision":"wait","action_ids":["a"],"rationale":"valid rationale"}),
        json!({"decision":"recovery","recovery_kind":"reobserve","action_ids":["a"],"rationale":"valid rationale"}),
        json!({"decision":"plan","action_ids":(0..9).map(|i|format!("a{i}")).collect::<Vec<_>>(),"rationale":"valid rationale"}),
    ] {
        assert!(parse_decision(&serde_json::to_vec(&value).expect("JSON")).is_err());
    }
    assert!(parse_decision(br#"{"decision":"plan","action_ids":["a"],"action_ids":["b"],"rationale":"valid rationale"}"#).is_err());
}

#[test]
fn plan_must_bind_every_proposed_action_before_dispatching_the_first() {
    let (mut source, _) = source(&["choice-1-a", "invented"]);
    assert_eq!(
        source.decide(&input(observation(false, 1))),
        Err(PolicyError::IllegalAction)
    );
}
