// SPDX-License-Identifier: MIT

fn shape_intent(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let fields = if kind == "attack" {
        &["kind", "damage", "hits", "target_ids"][..]
    } else if matches!(kind, "defend" | "buff" | "debuff" | "unknown") {
        &["kind", "target_ids"][..]
    } else if kind == "composite" {
        &["kind", "intents", "target_ids"][..]
    } else {
        return false;
    };
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return false;
    }
    if kind == "composite" {
        object
            .get("intents")
            .and_then(Value::as_array)
            .is_some_and(|intents| {
                intents.len() >= 2
                    && intents.len() <= MAX_TARGETS
                    && intents.iter().all(shape_intent_component)
            })
    } else {
        true
    }
}

fn shape_intent_component(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let fields = if kind == "attack" {
        &["kind", "damage", "hits", "target_ids"][..]
    } else if matches!(kind, "defend" | "buff" | "debuff" | "unknown") {
        &["kind", "target_ids"][..]
    } else {
        return false;
    };
    object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
}

fn shape_legal_actions(value: Option<&Value>) -> bool {
    let Some(items) = value.and_then(Value::as_array) else {
        return false;
    };
    items.iter().all(|item| {
        let Some(object) = exact_object(Some(item), &["action_id", "action"]) else {
            return false;
        };
        shape_action(object.get("action"))
    })
}

fn shape_action(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let Some(kind) = object.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let fields: &[&str] = match kind {
        "start_run" | "select_character" => &["kind", "character_id"],
        "select_map_node" => &["kind", "node_id"],
        "play_card" => &["kind", "card_id", "target_id"],
        "use_potion" => &["kind", "potion_id", "target_id"],
        "end_turn" | "skip_reward" | "proceed" | "rest" | "confirm_victory" | "save_quit" => {
            &["kind"]
        }
        "rest_option" => &["kind", "rest_option_id"],
        "choose_reward" => &["kind", "reward_id"],
        "shop_purchase" => &["kind", "item_id"],
        "shop_remove" | "smith" => &["kind", "card_id"],
        "event_choice" => &["kind", "choice_id"],
        "select_card" => &["kind", "selection_id", "card_id"],
        "confirm_selection" | "cancel_selection" => &["kind", "selection_id"],
        _ => return false,
    };
    object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
}
