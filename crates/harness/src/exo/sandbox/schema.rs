// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{SandboxError, ValueKind};

/// Fields of one entry in an offered set. `Choice` adds `contents`; an entry inside `contents` does
/// not, which is what keeps disclosure one level deep.
const CHOICE_CONTENT_FIELDS: &[&str] = &[
    "choice_id",
    "name",
    "cost",
    "upgraded",
    "description",
    "rarity",
];
const CHOICE_FIELDS: &[&str] = &[
    "choice_id",
    "name",
    "cost",
    "upgraded",
    "description",
    "rarity",
    "contents",
];
const CHOICE_OPTIONAL: &[&str] = &["name", "cost", "upgraded", "description", "rarity"];

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
            "hp",
            "max_hp",
            "energy",
            "gold",
            "hand",
            "deck",
            "discard",
            "exhaust",
            // What the player is carrying. A relic changes the rules for the whole run and a potion
            // is an action available this turn, so a model told neither is reasoning about a
            // different game from the one being played.
            "relics",
            "potions",
            "potion_slots",
            "max_potion_slots",
        ],
        // `description` is the host's own card text. It is admitted so a host that carries it can
        // say what a card does; a host that does not is unaffected, because absence is allowed.
        ValueKind::Card => &["card_id", "name", "cost", "upgraded", "description"],
        // Names follow the runtime-v4 expert shapes in runtime_v4_expert_shape_collections.rs, so
        // the two protocols describe the same things the same way. `description` is added to both,
        // because neither carried what a relic or a potion actually does.
        ValueKind::Relic => &["relic_id", "name", "description"],
        ValueKind::Potion => &[
            "potion_id",
            "name",
            "slot",
            "usable",
            "target_mode",
            "description",
        ],
        // An offered card or reward, when the host describes one rather than naming it.
        // `contents` is what taking this option would present next. A reward is chosen on one
        // screen and its contents on the following one, so without it the first choice is blind: a
        // card reward is only an identifier until it has already been taken.
        ValueKind::Choice => CHOICE_FIELDS,
        ValueKind::ChoiceContent => CHOICE_CONTENT_FIELDS,
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
            // `continue_run` names the saved run only when more than one can be resumed, so the
            // discriminator is admitted like an optional field rather than required.
            "run_id",
            "node_id",
            "card_id",
            "player_id",
            "target_id",
            "reward_id",
            "item_id",
            "choice_id",
            "potion_id",
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
        | (ValueKind::Player, "gold")
        | (ValueKind::Player, "potion_slots")
        | (ValueKind::Player, "max_potion_slots") => ValueKind::Number,
        (ValueKind::Player, "relics") => ValueKind::Relic,
        (ValueKind::Player, "potions") => ValueKind::Potion,
        (ValueKind::Relic, "relic_id") => ValueKind::Identity,
        (ValueKind::Relic, "name") | (ValueKind::Relic, "description") => ValueKind::Text,
        (ValueKind::Potion, "potion_id") => ValueKind::Identity,
        (ValueKind::Potion, "name")
        | (ValueKind::Potion, "description")
        | (ValueKind::Potion, "target_mode") => ValueKind::Text,
        (ValueKind::Potion, "slot") => ValueKind::Number,
        (ValueKind::Potion, "usable") => ValueKind::Boolean,
        (ValueKind::Card, "card_id") => ValueKind::Identity,
        (ValueKind::Card, "name") | (ValueKind::Card, "description") => ValueKind::Text,
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
        (ValueKind::State, "characters") => ValueKind::Identity,
        // An offered set: identifiers today, described objects when a host carries the detail.
        (ValueKind::State, "options") | (ValueKind::State, "choices") => ValueKind::Choice,
        (ValueKind::Choice, "contents") => ValueKind::ChoiceContent,
        (ValueKind::Choice, "choice_id") | (ValueKind::ChoiceContent, "choice_id") => {
            ValueKind::Identity
        }
        (ValueKind::ChoiceContent, "name")
        | (ValueKind::ChoiceContent, "description")
        | (ValueKind::ChoiceContent, "rarity") => ValueKind::Text,
        (ValueKind::ChoiceContent, "cost") => ValueKind::Number,
        (ValueKind::ChoiceContent, "upgraded") => ValueKind::Boolean,
        (ValueKind::Choice, "name")
        | (ValueKind::Choice, "description")
        | (ValueKind::Choice, "rarity") => ValueKind::Text,
        (ValueKind::Choice, "cost") => ValueKind::Number,
        (ValueKind::Choice, "upgraded") => ValueKind::Boolean,
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
        ValueKind::Player => require_fields(
            object,
            &[
                "hp", "max_hp", "energy", "gold", "hand", "deck", "discard", "exhaust",
            ],
            &["relics", "potions", "potion_slots", "max_potion_slots"],
        ),
        ValueKind::Card => require_fields(
            object,
            &["card_id", "name", "cost", "upgraded"],
            &["description"],
        ),
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
        ValueKind::Relic => require_fields(object, &["relic_id", "name"], &["description"]),
        ValueKind::Potion => require_fields(
            object,
            &["potion_id", "name"],
            &["slot", "usable", "target_mode", "description"],
        ),
        ValueKind::Choice => require_fields(object, &["choice_id"], &CHOICE_FIELDS[1..]),
        ValueKind::ChoiceContent => require_fields(object, &["choice_id"], CHOICE_OPTIONAL),
        ValueKind::LegalAction => require_exact(object, &["action_id", "action"]),
        ValueKind::Action => match object.get("kind").and_then(Value::as_str) {
            Some("start_run") => require_exact(object, &["kind", "character_id"]),
            // The host names a run only when the choice is not already determined, so `run_id` is
            // an optional host-owned identity; it must still be an identity, and an unknown key is
            // still refused. Recorded in `docs/ARCHITECTURE.md` (`sts2-harness#415` merge
            // `551ec19d`, agreed in `sts2-game-mod#210` merge `8a655143`).
            Some("continue_run") => require_fields(object, &["kind"], &["run_id"]),
            Some("select_map_node") => require_exact(object, &["kind", "node_id"]),
            Some("play_card") => require_exact(object, &["kind", "card_id", "target_id"]),
            Some("choose_reward") => require_exact(object, &["kind", "reward_id"]),
            Some("shop_purchase") => require_exact(object, &["kind", "item_id"]),
            Some("shop_remove" | "smith" | "select_card") => {
                require_exact(object, &["kind", "card_id"])
            }
            Some("use_potion") => require_exact(object, &["kind", "potion_id", "target_id"]),
            Some("discard_potion") => require_exact(object, &["kind", "potion_id"]),
            Some("select_player") => require_exact(object, &["kind", "player_id"]),
            Some("event_choice") => require_exact(object, &["kind", "choice_id"]),
            Some("end_turn" | "skip_reward" | "rest" | "confirm_victory" | "save_quit") => {
                require_exact(object, &["kind"])
            }
            Some("proceed" | "confirm_selection" | "cancel_selection") => {
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

/// Admits an object carrying every required field, and nothing beyond the optional ones.
///
/// `require_exact` counts keys, so it cannot express an optional field: an object carrying one is
/// refused for having the wrong number of them. A host that knows more than the minimum should not
/// have to withhold it, and a host that knows only the minimum should not have to invent the rest.
pub(super) fn require_fields(
    object: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
) -> Result<(), SandboxError> {
    if required.iter().all(|field| object.contains_key(*field))
        && object
            .keys()
            .all(|key| required.contains(&key.as_str()) || optional.contains(&key.as_str()))
    {
        Ok(())
    } else {
        Err(SandboxError::UnknownField)
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
