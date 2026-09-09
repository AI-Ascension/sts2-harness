// SPDX-License-Identifier: MIT

fn shape_cards(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            exact_object(
                Some(item),
                &[
                    "card_id",
                    "name",
                    "cost",
                    "upgraded",
                    "type",
                    "rarity",
                    "target",
                    "description",
                ],
            )
            .is_some()
        })
    })
}
fn shape_statuses(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| exact_object(Some(item), &["status_id", "name", "amount"]).is_some())
    })
}

fn shape_relics(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| exact_object(Some(item), &["relic_id", "name"]).is_some())
    })
}

fn shape_potions(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            exact_object(
                Some(item),
                &["potion_id", "name", "slot", "usable", "target_mode"],
            )
            .is_some()
        })
    })
}

fn shape_state(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    let Some(state) = object.get("state").and_then(Value::as_str) else {
        return false;
    };
    let fields: &[&str] = match state {
        "setup" => &["state", "characters"],
        "map" => &["state", "current_node_id", "nodes", "edges", "options"],
        "combat" => &["state", "turn_index", "enemies"],
        "reward" | "event" | "rest" | "selection" => &["state", "choices"],
        "shop" => &["state", "items"],
        "victory" => &["state"],
        "defeat" => &["state", "reason"],
        "recovery" => &["state", "code"],
        _ => return false,
    };
    if object.len() != fields.len() || fields.iter().any(|field| !object.contains_key(*field)) {
        return false;
    }
    match state {
        "map" => {
            if !shape_map_nodes(object.get("nodes")) || !shape_map_edges(object.get("edges")) {
                return false;
            }
        }
        "combat" => {
            if !shape_enemies(object.get("enemies")) {
                return false;
            }
        }
        "reward" | "event" | "rest" | "selection" => {
            if !shape_choices(object.get("choices")) {
                return false;
            }
        }
        "shop" => return shape_shop_items(object.get("items")),
        _ => {}
    }
    true
}

fn shape_array(value: Option<&Value>, fields: &[&str]) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items
            .iter()
            .all(|item| exact_object(Some(item), fields).is_some())
    })
}

fn shape_map_nodes(value: Option<&Value>) -> bool {
    shape_array(
        value,
        &["node_id", "act", "row", "col", "kind", "reachable"],
    )
}
fn shape_map_edges(value: Option<&Value>) -> bool {
    shape_array(value, &["from", "to"])
}
fn shape_choices(value: Option<&Value>) -> bool {
    shape_array(value, &["choice_id", "label", "kind", "domain"])
}
fn shape_shop_items(value: Option<&Value>) -> bool {
    shape_array(value, &["item_id", "name", "kind", "price"])
}

fn shape_enemies(value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    if value.is_null() {
        return true;
    }
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            let Some(enemy) = exact_object(
                Some(item),
                &[
                    "enemy_id", "name", "hp", "max_hp", "block", "powers", "statuses", "intent",
                ],
            ) else {
                return false;
            };
            shape_statuses(enemy.get("powers"))
                && shape_statuses(enemy.get("statuses"))
                && shape_intent(enemy.get("intent"))
        })
    })
}
