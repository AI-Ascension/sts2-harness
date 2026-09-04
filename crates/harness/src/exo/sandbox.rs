// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::{Map, Value};

const MAX_OBSERVATION_BYTES: usize = 128 * 1024;
const MAX_TEXT_BYTES: usize = 512;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_CARDS: usize = 256;
const MAX_ENEMIES: usize = 64;
const MAX_LEGAL_ACTIONS: usize = 256;
const MAX_SHOP_ITEMS: usize = 128;
const MAX_TEXT_ITEMS: usize = 256;

/// JSON projection that admits only ordinary player-visible fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedObservation(Value);

impl SanitizedObservation {
    /// Validates a projection before it enters an Exo request.
    pub fn new(value: Value) -> Result<Self, SandboxError> {
        let encoded = serde_json::to_vec(&value).map_err(|_| SandboxError::MalformedJson)?;
        if encoded.len() > MAX_OBSERVATION_BYTES {
            return Err(SandboxError::TooLarge);
        }
        validate_value(&value, ValueKind::Root, true)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    #[must_use]
    pub fn state_id(&self) -> Option<&str> {
        self.0.get("state_id").and_then(Value::as_str)
    }

    #[must_use]
    pub fn generation(&self) -> Option<u64> {
        self.0.get("generation").and_then(Value::as_u64)
    }
}

/// A rejected projection never reaches a provider transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxError {
    MalformedJson,
    TooLarge,
    NotAnObservation,
    UnknownField,
    PrivilegedField,
    InvalidText,
    InvalidNumber,
    InvalidCollection,
    DuplicateLegalAction,
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MalformedJson => "fair-play observation could not be encoded",
            Self::TooLarge => "fair-play observation exceeds its byte bound",
            Self::NotAnObservation => {
                "fair-play observation must be an object with state and actions"
            }
            Self::UnknownField => "fair-play observation contains an unknown field",
            Self::PrivilegedField => "fair-play observation contains a privileged field",
            Self::InvalidText => "fair-play observation contains invalid text",
            Self::InvalidNumber => "fair-play observation contains an invalid number",
            Self::InvalidCollection => "fair-play observation contains an oversized collection",
            Self::DuplicateLegalAction => "fair-play observation contains a duplicate legal action",
        })
    }
}

impl std::error::Error for SandboxError {}

#[derive(Clone, Copy)]
enum ValueKind {
    Root,
    Player,
    Card,
    Enemy,
    Intent,
    State,
    ShopItem,
    LegalAction,
    Action,
    Identity,
    Text,
    Number,
    Boolean,
}

fn validate_value(value: &Value, kind: ValueKind, root: bool) -> Result<(), SandboxError> {
    match value {
        Value::Object(object)
            if !matches!(
                kind,
                ValueKind::Identity | ValueKind::Text | ValueKind::Number | ValueKind::Boolean
            ) =>
        {
            validate_object(object, kind, root)
        }
        Value::Object(_) => Err(SandboxError::NotAnObservation),
        Value::Array(_) => Err(SandboxError::InvalidCollection),
        Value::String(text) => match kind {
            ValueKind::Text if valid_text(text) => Ok(()),
            ValueKind::Identity if valid_identity(text) => Ok(()),
            _ => Err(SandboxError::InvalidText),
        },
        Value::Number(number) => {
            if matches!(kind, ValueKind::Number)
                && number
                    .as_u64()
                    .is_some_and(|value| value <= MAX_SAFE_INTEGER)
            {
                Ok(())
            } else {
                Err(SandboxError::InvalidNumber)
            }
        }
        Value::Bool(_) if matches!(kind, ValueKind::Boolean) => Ok(()),
        Value::Bool(_) => Err(SandboxError::InvalidNumber),
        Value::Null => Err(SandboxError::NotAnObservation),
    }
}

fn validate_object(
    object: &Map<String, Value>,
    kind: ValueKind,
    root: bool,
) -> Result<(), SandboxError> {
    for (key, value) in object {
        if is_privileged_key(key) {
            return Err(SandboxError::PrivilegedField);
        }
        if !is_allowed(kind, key) {
            return Err(SandboxError::UnknownField);
        }
        if matches!(value, Value::Null) && allows_null(kind, key) {
            continue;
        }
        if let Value::Number(number) = value {
            validate_number_bound(kind, key, number)?;
        }
        let child = child_kind(kind, key);
        if let Some(maximum) = collection_bound(kind, key) {
            let values = value.as_array().ok_or(SandboxError::InvalidCollection)?;
            if values.len() > maximum {
                return Err(SandboxError::InvalidCollection);
            }
            for item in values {
                validate_value(item, child, false)?;
            }
        } else {
            validate_value(value, child, false)?;
        }
    }
    if root {
        require_exact(
            object,
            &[
                "state_id",
                "generation",
                "visible_seed",
                "player",
                "state",
                "legal_actions",
            ],
        )?;
    }
    validate_shape(object, kind, root)?;
    validate_semantics(object, kind)?;
    Ok(())
}

fn collection_bound(kind: ValueKind, key: &str) -> Option<usize> {
    match (kind, key) {
        (ValueKind::Player, "hand" | "deck" | "discard" | "exhaust") => Some(MAX_CARDS),
        (ValueKind::State, "enemies") => Some(MAX_ENEMIES),
        (ValueKind::Root, "legal_actions") => Some(MAX_LEGAL_ACTIONS),
        (ValueKind::State, "items") => Some(MAX_SHOP_ITEMS),
        (ValueKind::State, "characters" | "options" | "choices") => Some(MAX_TEXT_ITEMS),
        _ => None,
    }
}

fn validate_number_bound(
    kind: ValueKind,
    key: &str,
    value: &serde_json::Number,
) -> Result<(), SandboxError> {
    let maximum = match (kind, key) {
        (ValueKind::Root, "generation") => MAX_SAFE_INTEGER,
        (ValueKind::Player, "hp" | "max_hp") => 65_535,
        (ValueKind::Player, "energy") => 255,
        (ValueKind::Player, "gold") => 4_294_967_295,
        (ValueKind::Card, "cost") => 255,
        (ValueKind::Enemy, "hp" | "max_hp") => 65_535,
        (ValueKind::Intent, "damage") => 65_535,
        (ValueKind::Intent, "hits") => 255,
        (ValueKind::State, "turn_index") => 65_535,
        (ValueKind::ShopItem, "price") => 4_294_967_295,
        _ => MAX_SAFE_INTEGER,
    };
    if value.as_u64().is_some_and(|number| number <= maximum) {
        Ok(())
    } else {
        Err(SandboxError::InvalidNumber)
    }
}

fn validate_semantics(object: &Map<String, Value>, kind: ValueKind) -> Result<(), SandboxError> {
    match kind {
        ValueKind::Root => {
            let Some(actions) = object.get("legal_actions").and_then(Value::as_array) else {
                return Err(SandboxError::NotAnObservation);
            };
            let mut action_ids = BTreeSet::new();
            for action in actions {
                let Some(action_id) = action.get("action_id").and_then(Value::as_str) else {
                    return Err(SandboxError::NotAnObservation);
                };
                if !action_ids.insert(action_id) {
                    return Err(SandboxError::DuplicateLegalAction);
                }
            }
        }
        ValueKind::Player | ValueKind::Enemy => {
            let hp = object.get("hp").and_then(Value::as_u64);
            let max_hp = object.get("max_hp").and_then(Value::as_u64);
            if hp.zip(max_hp).is_some_and(|(hp, max_hp)| hp > max_hp) {
                return Err(SandboxError::InvalidNumber);
            }
        }
        ValueKind::Intent
            if object.get("kind").and_then(Value::as_str) == Some("attack")
                && object.get("hits").and_then(Value::as_u64) == Some(0) =>
        {
            return Err(SandboxError::InvalidNumber);
        }
        _ => {}
    }
    Ok(())
}

fn allows_null(kind: ValueKind, key: &str) -> bool {
    matches!(
        (kind, key),
        (ValueKind::Root, "visible_seed")
            | (ValueKind::State, "node_id")
            | (ValueKind::State, "reason")
            | (ValueKind::Action, "target_id")
    )
}

fn is_allowed(kind: ValueKind, key: &str) -> bool {
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

fn child_kind(parent: ValueKind, key: &str) -> ValueKind {
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

fn validate_shape(
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

fn require_exact(object: &Map<String, Value>, fields: &[&str]) -> Result<(), SandboxError> {
    if object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field)) {
        Ok(())
    } else {
        Err(SandboxError::UnknownField)
    }
}

fn is_privileged_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "rng",
        "random_state",
        "future",
        "unrevealed",
        "secret",
        "credential",
        "password",
        "access_token",
        "raw_memory",
        "host_object",
        "executable",
        "pck",
        "dll",
        "save_file",
        "process_command",
        "reflection",
        "screen_coordinate",
        "input_event",
        "private_prompt",
    ]
    .iter()
    .any(|forbidden| key == *forbidden || key.contains(forbidden))
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TEXT_BYTES && !value.chars().any(char::is_control)
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
