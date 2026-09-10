// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sts2_harness::{EpisodeLegalActionSet, EpisodeObservation, EpisodeStage};

use super::config::RuntimeConfig;

#[path = "runtime_v3_parse_catalog.rs"]
mod catalog;
pub(crate) use catalog::action_from_payload;

#[path = "runtime_v3_parse_transition.rs"]
mod transition;

#[path = "runtime_v3_parse_observation.rs"]
mod observation;
#[cfg(test)]
pub(super) use observation::{action_set, observation, result_observation};
pub(super) use observation::{
    action_set_with_catalog_text, observation_from_root, observation_with_text,
    result_observation_with_text,
};

#[cfg(test)]
#[path = "runtime_v3_parse_catalog_tests.rs"]
mod catalog_tests;
#[cfg(test)]
#[path = "runtime_v3_parse_test.rs"]
mod tests;

pub(super) use transition::{receipt, wait_sample};

const PROTOCOL_VERSION: &str = "runtime-v3-gameplay";
const SCHEMA_DIGEST: &str = "8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63";
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
    /// The exact legal_actions JSON value as received from the authoritative host. Recovery
    /// binds an operation to this value's bytes; callers must not replace it with the parsed
    /// payload map because that changes the catalog digest and loses wire shape.
    pub(super) catalog: Value,
    pub(super) catalog_raw: Vec<u8>,
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
    validate_metadata(object)?;
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
    number(object, "generation")?;
    if !object
        .get("state_id")
        .is_some_and(|value| value.is_null() || value.as_str().is_some_and(safe_identity))
    {
        return Err(String::from("Runtime-v3 state identity is invalid"));
    }
    Ok(object)
}

fn validate_metadata(object: &Map<String, Value>) -> Result<(), String> {
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
    Ok(())
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
