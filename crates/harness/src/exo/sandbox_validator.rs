// SPDX-License-Identifier: MIT

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
        require_root(object)?;
    }
    validate_shape(object, kind, root)?;
    validate_semantics(object, kind)?;
    Ok(())
}

/// The root carries exactly the five required fair-play fields; `visible_seed` is optional
/// so seed-blind experiments can explicitly omit it (see `without_visible_seed`).
fn require_root(object: &Map<String, Value>) -> Result<(), SandboxError> {
    const REQUIRED: [&str; 5] = ["state_id", "generation", "player", "state", "legal_actions"];
    let expected = REQUIRED.len() + usize::from(object.contains_key("visible_seed"));
    if object.len() == expected && REQUIRED.iter().all(|field| object.contains_key(*field)) {
        Ok(())
    } else {
        Err(SandboxError::UnknownField)
    }
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
