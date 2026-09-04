// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sts2_harness::{
    ActionKind, ActionSetError, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeStage,
};

use super::config::RuntimeConfig;

#[path = "runtime_v3_parse_transition.rs"]
mod transition;

#[cfg(test)]
#[path = "runtime_v3_parse_test.rs"]
mod tests;

pub(super) use transition::{receipt, wait_sample};

const PROTOCOL_VERSION: &str = "runtime-v3-gameplay";
const SCHEMA_DIGEST: &str = "fbfb18279b0c7ebb350ef0ce0d56547fa11e83985b13380cb2b0f1dba4cb56e9";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const ROOT_FIELDS: [&str; 21] = [
    "protocol_version",
    "schema_digest",
    "provenance",
    "correlation_id",
    "instance_id",
    "session_id",
    "lease_id",
    "lease_epoch",
    "generation",
    "kind",
    "state_id",
    "operation_id",
    "observation",
    "legal_actions",
    "action",
    "status",
    "transition",
    "error_code",
    "wait_for_millis",
    "wait_outcome",
    "recovery",
];

pub(super) struct ParsedObservation {
    pub(super) observation: EpisodeObservation,
    pub(super) actions: EpisodeLegalActionSet,
    pub(super) payloads: BTreeMap<String, Value>,
}

pub(super) fn observation(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<ParsedObservation, String> {
    let root = root(value, expected_kind, config)?;
    validate_observation_fields(root)?;
    observation_from_root(root)
}

pub(super) fn action_set(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<(EpisodeLegalActionSet, BTreeMap<String, Value>), String> {
    let root = root(value, expected_kind, config)?;
    validate_observation_fields(root)?;
    require_null(root, "observation")?;
    let state_id = string(root, "state_id")?;
    let generation = number(root, "generation")?;
    parse_actions(root.get("legal_actions"), state_id, generation)
}

// Installation of an already validated receipt/wait uses its result shape, not the
// all-null status/operation shape of a standalone observation response.
pub(super) fn result_observation(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<ParsedObservation, String> {
    if !matches!(
        expected_kind,
        "dispatch_action_response" | "wait_response" | "recover_response"
    ) {
        return Err(String::from(
            "Runtime-v3 installation requires a result response",
        ));
    }
    let root = root(value, expected_kind, config)?;
    transition::validate_installation_fields(root, expected_kind == "wait_response")?;
    observation_from_root(root)
}

fn root<'a>(
    value: &'a Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| String::from("Runtime-v3 MCP content was not an object"))?;
    if object.len() != ROOT_FIELDS.len()
        || ROOT_FIELDS.iter().any(|field| !object.contains_key(*field))
    {
        return Err(String::from(
            "Runtime-v3 MCP content has an invalid root shape",
        ));
    }
    if object.get("protocol_version").and_then(Value::as_str) != Some(PROTOCOL_VERSION)
        || object.get("schema_digest").and_then(Value::as_str) != Some(SCHEMA_DIGEST)
    {
        return Err(String::from(
            "Runtime-v3 MCP content has unsupported metadata",
        ));
    }
    let Some(provenance) = object.get("provenance").and_then(Value::as_object) else {
        return Err(String::from("Runtime-v3 MCP content omitted provenance"));
    };
    if provenance.len() != 3
        || provenance.get("artifact").and_then(Value::as_str)
            != Some("sts2-protocol/runtime-v3-gameplay")
        || provenance.get("source").and_then(Value::as_str)
            != Some("schemas/runtime-v3-gameplay.schema.json")
        || provenance.get("generator").and_then(Value::as_str) != Some("hand-authored")
    {
        return Err(String::from("Runtime-v3 MCP provenance is unsupported"));
    }
    for (field, expected) in [
        ("instance_id", config.instance_id.as_str()),
        ("session_id", config.session_id.as_str()),
        ("lease_id", config.lease_id.as_str()),
    ] {
        if object.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(String::from(
                "Runtime-v3 MCP identity does not match configuration",
            ));
        }
    }
    if number(object, "lease_epoch")? != config.lease_epoch
        || object.get("kind").and_then(Value::as_str) != Some(expected_kind)
        || !object
            .get("correlation_id")
            .and_then(Value::as_str)
            .is_some_and(safe_identity)
    {
        return Err(String::from(
            "Runtime-v3 MCP identity or kind does not match",
        ));
    }
    Ok(object)
}

fn observation_from_root(root: &Map<String, Value>) -> Result<ParsedObservation, String> {
    let state_id = string(root, "state_id")?;
    let generation = number(root, "generation")?;
    let raw_observation = root
        .get("observation")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("Runtime-v3 response omitted observation"))?;
    let observation = Value::Object(raw_observation.clone());
    if raw_observation.len() != 5
        || ["state_id", "generation", "visible_seed", "player", "state"]
            .iter()
            .any(|field| !raw_observation.contains_key(*field))
        || raw_observation.get("state_id").and_then(Value::as_str) != Some(state_id)
        || raw_observation.get("generation").and_then(Value::as_u64) != Some(generation)
    {
        return Err(String::from(
            "Runtime-v3 response observation identity is inconsistent",
        ));
    }
    let (actions, payloads) = parse_actions(root.get("legal_actions"), state_id, generation)?;
    let mut fair_play = raw_observation.clone();
    fair_play.insert(
        String::from("legal_actions"),
        root.get("legal_actions")
            .cloned()
            .ok_or_else(|| String::from("Runtime-v3 response omitted legal_actions"))?,
    );
    let stage = stage(&observation)?;
    let actionable = stage.is_actionable() && !actions.actions().is_empty();
    let episode_observation = EpisodeObservation::new(
        state_id,
        generation,
        stage,
        actionable,
        !stage.is_actionable(),
        actionable,
        Value::Object(fair_play),
    )
    .map_err(|error| format!("fair-play observation failed validation: {error}"))?;
    Ok(ParsedObservation {
        observation: episode_observation,
        actions,
        payloads,
    })
}

fn validate_observation_fields(root: &Map<String, Value>) -> Result<(), String> {
    for field in [
        "operation_id",
        "action",
        "status",
        "transition",
        "error_code",
        "wait_for_millis",
        "wait_outcome",
        "recovery",
    ] {
        require_null(root, field)?;
    }
    Ok(())
}

fn parse_actions(
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

fn stage(value: &Value) -> Result<EpisodeStage, String> {
    match value
        .get("state")
        .and_then(Value::as_object)
        .and_then(|object| object.get("state"))
        .and_then(Value::as_str)
    {
        Some("setup") => Ok(EpisodeStage::Setup),
        Some("map") => Ok(EpisodeStage::Map),
        Some("combat") => Ok(EpisodeStage::Combat),
        Some("reward") => Ok(EpisodeStage::Reward),
        Some("shop") => Ok(EpisodeStage::Shop),
        Some("event") => Ok(EpisodeStage::Event),
        Some("rest") => Ok(EpisodeStage::Rest),
        Some("selection") => Ok(EpisodeStage::Selection),
        Some("victory") => Ok(EpisodeStage::Victory),
        Some("defeat") => Ok(EpisodeStage::Defeat),
        Some("recovery") => Ok(EpisodeStage::Recovery),
        _ => Err(String::from("Runtime-v3 observation state is unknown")),
    }
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

fn string<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| safe_identity(value))
        .ok_or_else(|| format!("Runtime-v3 {field} is invalid"))
}

fn number(object: &Map<String, Value>, field: &str) -> Result<u64, String> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_SAFE_INTEGER)
        .ok_or_else(|| format!("Runtime-v3 {field} is invalid"))
}

fn require_null(object: &Map<String, Value>, field: &str) -> Result<(), String> {
    if object.get(field).is_some_and(Value::is_null) {
        Ok(())
    } else {
        Err(format!("Runtime-v3 {field} must be null"))
    }
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn action_set_error(error: ActionSetError) -> String {
    error.to_string()
}
