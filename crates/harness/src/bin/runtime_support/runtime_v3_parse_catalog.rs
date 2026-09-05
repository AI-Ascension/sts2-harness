// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::Value;
use sts2_harness::{ActionKind, ActionSetError, EpisodeLegalAction, EpisodeLegalActionSet};

use super::safe_identity;

pub(super) fn parse_actions(
    value: Option<&Value>,
    state_id: &str,
    generation: u64,
) -> Result<(EpisodeLegalActionSet, BTreeMap<String, Value>), String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("Runtime-v3 response omitted legal_actions"))?;
    let mut actions = Vec::with_capacity(values.len());
    let mut payloads = BTreeMap::new();
    for value in values {
        let object = value
            .as_object()
            .ok_or_else(|| String::from("Runtime-v3 legal action is not an object"))?;
        if object.len() != 2 || !object.contains_key("action_id") || !object.contains_key("action")
        {
            return Err(String::from("Runtime-v3 legal action has an invalid shape"));
        }
        let action_id = object
            .get("action_id")
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("Runtime-v3 legal action_id is invalid"))?;
        let payload = object
            .get("action")
            .ok_or_else(|| String::from("Runtime-v3 legal action omitted payload"))?;
        let kind = payload
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("Runtime-v3 legal action kind is invalid"))?;
        validate_payload(payload, kind)?;
        let action =
            EpisodeLegalAction::new(action_id, action_kind(kind)).map_err(action_set_error)?;
        payloads.insert(action_id.to_owned(), payload.clone());
        actions.push(action);
    }
    let action_set =
        EpisodeLegalActionSet::new(state_id, generation, actions).map_err(action_set_error)?;
    Ok((action_set, payloads))
}

fn validate_payload(value: &Value, kind: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| String::from("Runtime-v3 action payload is not an object"))?;
    let fields: &[&str] = match kind {
        "end_turn" | "skip_reward" | "rest" | "confirm_victory" | "save_quit" => &["kind"],
        "start_run" => &["kind", "character_id"],
        "select_map_node" => &["kind", "node_id"],
        "choose_reward" => &["kind", "reward_id"],
        "shop_purchase" => &["kind", "item_id"],
        "shop_remove" | "smith" | "select_card" => &["kind", "card_id"],
        "event_choice" => &["kind", "choice_id"],
        "play_card" => &["kind", "card_id", "target_id"],
        _ => return Err(String::from("Runtime-v3 action kind is not allowlisted")),
    };
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return Err(String::from(
            "Runtime-v3 action payload has an invalid field set",
        ));
    }
    if object.get("kind").and_then(Value::as_str) != Some(kind) {
        return Err(String::from("Runtime-v3 action kind is inconsistent"));
    }
    for field in fields.iter().copied().filter(|field| *field != "kind") {
        let valid = if field == "target_id" {
            object
                .get(field)
                .is_some_and(|value| value.is_null() || value.as_str().is_some_and(safe_identity))
        } else {
            object
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(safe_identity)
        };
        if !valid {
            return Err(String::from("Runtime-v3 action argument is invalid"));
        }
    }
    Ok(())
}

fn action_kind(kind: &str) -> ActionKind {
    match kind {
        "start_run" => ActionKind::StartRun,
        "select_map_node" => ActionKind::SelectMapNode,
        "play_card" => ActionKind::PlayCard,
        "end_turn" => ActionKind::EndTurn,
        "choose_reward" => ActionKind::ChooseReward,
        "skip_reward" => ActionKind::SkipReward,
        "shop_purchase" => ActionKind::ShopPurchase,
        "shop_remove" => ActionKind::ShopRemove,
        "rest" => ActionKind::Rest,
        "smith" => ActionKind::Smith,
        "event_choice" => ActionKind::EventChoice,
        "select_card" => ActionKind::SelectCard,
        "confirm_victory" => ActionKind::ConfirmVictory,
        "save_quit" => ActionKind::SaveQuit,
        _ => ActionKind::SaveQuit,
    }
}

fn action_set_error(error: ActionSetError) -> String {
    error.to_string()
}
