// SPDX-License-Identifier: MIT

use serde_json::{Map, Value, json};

use super::{privacy_digest, privacy_value_digest, required_str};

pub(super) fn observation_summary(observation: &Value) -> Result<Value, String> {
    let player = observation.get("player").and_then(Value::as_object);
    let legal_action_count = observation
        .get("legal_actions")
        .and_then(Value::as_array)
        .map(|actions| actions.len());
    let generation = observation
        .get("generation")
        .and_then(Value::as_u64)
        .ok_or_else(|| String::from("observation generation missing"))?;
    let state_id = required_str(observation, "state_id")?;
    let legal_action_count =
        legal_action_count.ok_or_else(|| String::from("observation legal_actions missing"))?;
    if legal_action_count > 25_000 {
        return Err(String::from("observation legal_actions exceeds bound"));
    }
    let mut player_summary = Map::new();
    for (target, value) in [
        (
            "hp",
            player
                .and_then(|value| value.get("hp"))
                .and_then(Value::as_u64),
        ),
        (
            "max_hp",
            player
                .and_then(|value| value.get("max_hp"))
                .and_then(Value::as_u64),
        ),
        (
            "energy",
            player
                .and_then(|value| value.get("energy"))
                .and_then(Value::as_u64),
        ),
        (
            "gold",
            player
                .and_then(|value| value.get("gold"))
                .and_then(Value::as_u64),
        ),
    ] {
        if let Some(value) = value {
            player_summary.insert(target.to_owned(), Value::String(value.to_string()));
        }
    }
    let mut summary = Map::new();
    summary.insert(
        "generation".to_owned(),
        Value::String(generation.to_string()),
    );
    summary.insert(
        "state_id_digest".to_owned(),
        Value::String(privacy_digest("state", state_id)),
    );
    summary.insert(
        "observation_digest".to_owned(),
        Value::String(privacy_value_digest("observation", observation)?),
    );
    summary.insert("legal_action_count".to_owned(), json!(legal_action_count));
    if !player_summary.is_empty() {
        summary.insert("player".to_owned(), Value::Object(player_summary));
    }
    Ok(Value::Object(summary))
}
