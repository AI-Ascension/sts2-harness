// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{SandboxError, ValueKind};

pub(super) fn allows_null(kind: ValueKind, key: &str) -> bool {
    matches!(
        (kind, key),
        (ValueKind::Root, "visible_seed")
            | (ValueKind::State, "node_id")
            | (ValueKind::State, "reason")
            | (ValueKind::Action, "target_id")
    )
}

pub(super) fn is_allowed(kind: ValueKind, key: &str) -> bool {
    let keys: &[&str] = match kind {
        ValueKind::Root => &[
            "state_id",
            "generation",
            "visible_seed",
            "player",
            "state",
            "legal_actions",
        ],
        ValueKind::Player => &[
            "hp", "max_hp", "energy", "gold", "hand", "deck", "discard", "exhaust",
        ],
        ValueKind::Card => &["card_id", "name", "cost", "upgraded"],
        ValueKind::Enemy => &["enemy_id", "name", "hp", "max_hp", "intent"],
        ValueKind::Intent => &["kind", "damage", "hits"],
        ValueKind::State => &[
            "state",
            "characters",
            "node_id",
            "options",
            "turn_index",
            "enemies",
            "choices",
            "items",
            "reason",
            "code",
        ],
        ValueKind::ShopItem => &["item_id", "name", "price"],
        ValueKind::LegalAction => &["action_id", "action"],
        ValueKind::Action => &[
            "kind",
            "character_id",
            "node_id",
            "card_id",
            "target_id",
            "reward_id",
            "item_id",
            "choice_id",
        ],
        ValueKind::Identity | ValueKind::Text | ValueKind::Number | ValueKind::Boolean => &[],
    };
    keys.contains(&key)
}

pub(super) fn child_kind(parent: ValueKind, key: &str) -> ValueKind {
    match (parent, key) {
        (ValueKind::Root, "player") => ValueKind::Player,
        (ValueKind::Root, "state") => ValueKind::State,
        (ValueKind::Root, "legal_actions") => ValueKind::LegalAction,
        (ValueKind::Root, "state_id") => ValueKind::Identity,
        (ValueKind::Root, "visible_seed") => ValueKind::Text,
        (ValueKind::Root, "generation") => ValueKind::Number,
        (ValueKind::Player, "hand")
        | (ValueKind::Player, "deck")
        | (ValueKind::Player, "discard")
        | (ValueKind::Player, "exhaust") => ValueKind::Card,
        (ValueKind::Player, "hp")
        | (ValueKind::Player, "max_hp")
        | (ValueKind::Player, "energy")
        | (ValueKind::Player, "gold") => ValueKind::Number,
        (ValueKind::Card, "card_id") => ValueKind::Identity,
        (ValueKind::Card, "name") => ValueKind::Text,
        (ValueKind::Card, "cost") => ValueKind::Number,
        (ValueKind::Card, "upgraded") => ValueKind::Boolean,
        (ValueKind::Enemy, "enemy_id") => ValueKind::Identity,
        (ValueKind::Enemy, "name") => ValueKind::Text,
        (ValueKind::Enemy, "hp") | (ValueKind::Enemy, "max_hp") => ValueKind::Number,
        (ValueKind::State, "enemies") => ValueKind::Enemy,
        (ValueKind::State, "items") => ValueKind::ShopItem,
        (ValueKind::State, "state")
        | (ValueKind::State, "node_id")
        | (ValueKind::State, "code") => ValueKind::Identity,
        (ValueKind::State, "characters")
        | (ValueKind::State, "options")
        | (ValueKind::State, "choices") => ValueKind::Identity,
        (ValueKind::State, "reason") => ValueKind::Text,
        (ValueKind::State, "turn_index") => ValueKind::Number,
        (ValueKind::Enemy, "intent") => ValueKind::Intent,
        (ValueKind::Intent, "damage") | (ValueKind::Intent, "hits") => ValueKind::Number,
        (ValueKind::ShopItem, "item_id") => ValueKind::Identity,
        (ValueKind::ShopItem, "name") => ValueKind::Text,
        (ValueKind::ShopItem, "price") => ValueKind::Number,
        (ValueKind::LegalAction, "action_id") => ValueKind::Identity,
        (ValueKind::LegalAction, "action") => ValueKind::Action,
        (ValueKind::Action, _) => ValueKind::Identity,
        (ValueKind::Intent, "kind") => ValueKind::Identity,
        _ => ValueKind::Text,
    }
}

pub(super) fn validate_shape(
    object: &Map<String, Value>,
    kind: ValueKind,
    root: bool,
) -> Result<(), SandboxError> {
    if root {
        return Ok(());
    }
    match kind {
        ValueKind::Player => require_exact(
            object,
            &[
                "hp", "max_hp", "energy", "gold", "hand", "deck", "discard", "exhaust",
            ],
        ),
        ValueKind::Card => require_exact(object, &["card_id", "name", "cost", "upgraded"]),
        ValueKind::Enemy => require_exact(object, &["enemy_id", "name", "hp", "max_hp", "intent"]),
        ValueKind::Intent => match object.get("kind").and_then(Value::as_str) {
            Some("attack") => require_exact(object, &["kind", "damage", "hits"]),
            Some("defend" | "buff" | "debuff" | "unknown") => require_exact(object, &["kind"]),
            _ => Err(SandboxError::UnknownField),
        },
        ValueKind::State => match object.get("state").and_then(Value::as_str) {
            Some("setup") => require_exact(object, &["state", "characters"]),
            Some("map") => require_exact(object, &["state", "node_id", "options"]),
            Some("combat") => require_exact(object, &["state", "turn_index", "enemies"]),
            Some("reward" | "rest") => require_exact(object, &["state", "options"]),
            Some("shop") => require_exact(object, &["state", "items"]),
            Some("event" | "selection") => require_exact(object, &["state", "choices"]),
            Some("victory") => require_exact(object, &["state"]),
            Some("defeat") => require_exact(object, &["state", "reason"]),
            Some("recovery") => require_exact(object, &["state", "code"]),
            _ => Err(SandboxError::UnknownField),
        },
        ValueKind::ShopItem => require_exact(object, &["item_id", "name", "price"]),
        ValueKind::LegalAction => require_exact(object, &["action_id", "action"]),
        ValueKind::Action => match object.get("kind").and_then(Value::as_str) {
            Some("start_run") => require_exact(object, &["kind", "character_id"]),
            Some("select_map_node") => require_exact(object, &["kind", "node_id"]),
            Some("play_card") => require_exact(object, &["kind", "card_id", "target_id"]),
            Some("choose_reward") => require_exact(object, &["kind", "reward_id"]),
            Some("shop_purchase") => require_exact(object, &["kind", "item_id"]),
            Some("shop_remove" | "smith" | "select_card") => {
                require_exact(object, &["kind", "card_id"])
            }
            Some("event_choice") => require_exact(object, &["kind", "choice_id"]),
            Some("end_turn" | "skip_reward" | "rest" | "confirm_victory" | "save_quit") => {
                require_exact(object, &["kind"])
            }
            _ => Err(SandboxError::UnknownField),
        },
        ValueKind::Root
        | ValueKind::Identity
        | ValueKind::Text
        | ValueKind::Number
        | ValueKind::Boolean => Ok(()),
    }
}

pub(super) fn require_exact(
    object: &Map<String, Value>,
    fields: &[&str],
) -> Result<(), SandboxError> {
    if object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field)) {
        Ok(())
    } else {
        Err(SandboxError::UnknownField)
    }
}
