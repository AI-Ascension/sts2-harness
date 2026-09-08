// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sts2_harness::{EpisodeLegalActionSet, EpisodeObservation};

use super::catalog::{parse_actions, raw_catalog};
use super::{
    ParsedObservation, RuntimeConfig, number, require_null, root, stage, string, transition,
    validate_observation_fields,
};

#[allow(dead_code)]
pub(in super::super) fn observation(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<ParsedObservation, String> {
    let root = root(value, expected_kind, config)?;
    validate_observation_fields(root)?;
    observation_from_root(root, None)
}

pub(in super::super) fn observation_with_text(
    value: &Value,
    response_text: &str,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<ParsedObservation, String> {
    let root = root(value, expected_kind, config)?;
    validate_observation_fields(root)?;
    observation_from_root(root, Some(response_text))
}

#[allow(dead_code)]
pub(in super::super) fn action_set(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<(EpisodeLegalActionSet, BTreeMap<String, Value>), String> {
    let (actions, payloads, _) = action_set_with_catalog(value, expected_kind, config)?;
    Ok((actions, payloads))
}

pub(in super::super) fn action_set_with_catalog(
    value: &Value,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<(EpisodeLegalActionSet, BTreeMap<String, Value>, Value), String> {
    let root = root(value, expected_kind, config)?;
    validate_observation_fields(root)?;
    require_null(root, "observation")?;
    let state_id = string(root, "state_id")?;
    let generation = number(root, "generation")?;
    let catalog = root
        .get("legal_actions")
        .cloned()
        .ok_or_else(|| String::from("Runtime-v3 response omitted legal_actions"))?;
    let (actions, payloads) = parse_actions(Some(&catalog), state_id, generation)?;
    Ok((actions, payloads, catalog))
}

pub(in super::super) struct ParsedActionCatalog {
    pub(in super::super) actions: EpisodeLegalActionSet,
    pub(in super::super) payloads: BTreeMap<String, Value>,
    pub(in super::super) catalog: Value,
    pub(in super::super) catalog_raw: Vec<u8>,
}

pub(in super::super) fn action_set_with_catalog_text(
    value: &Value,
    response_text: &str,
    expected_kind: &str,
    config: &RuntimeConfig,
) -> Result<ParsedActionCatalog, String> {
    let root = root(value, expected_kind, config)?;
    validate_observation_fields(root)?;
    require_null(root, "observation")?;
    let state_id = string(root, "state_id")?;
    let generation = number(root, "generation")?;
    let catalog = root
        .get("legal_actions")
        .cloned()
        .ok_or_else(|| String::from("Runtime-v3 response omitted legal_actions"))?;
    let catalog_raw = raw_catalog(Some(response_text), &catalog)?;
    let (actions, payloads) = parse_actions(Some(&catalog), state_id, generation)?;
    Ok(ParsedActionCatalog {
        actions,
        payloads,
        catalog,
        catalog_raw,
    })
}

// Installation of an already validated receipt/wait uses its result shape, not the
// all-null status/operation shape of a standalone observation response.
#[allow(dead_code)]
pub(in super::super) fn result_observation(
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
    observation_from_root(root, None)
}

pub(in super::super) fn result_observation_with_text(
    value: &Value,
    response_text: &str,
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
    observation_from_root(root, Some(response_text))
}

pub(in super::super) fn observation_from_root(
    root: &Map<String, Value>,
    response_text: Option<&str>,
) -> Result<ParsedObservation, String> {
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
    let catalog = root
        .get("legal_actions")
        .cloned()
        .ok_or_else(|| String::from("Runtime-v3 response omitted legal_actions"))?;
    let catalog_raw = raw_catalog(response_text, &catalog)?;
    let mut fair_play = raw_observation.clone();
    fair_play.insert(String::from("legal_actions"), catalog.clone());
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
        catalog,
        catalog_raw,
    })
}
