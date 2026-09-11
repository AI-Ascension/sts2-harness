// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{
    MAX_BINDINGS, MAX_GENERATION, MAX_HOST_ACTION_ID_BYTES, MAX_ID_BYTES, MAX_SNAPSHOT_BYTES,
    MAX_TEXT_BYTES, MapError, RUNTIME_MAP_PROFILE, VISIBLE_MAP_SCHEMA, graph, valid_identity,
    valid_identity_limit,
};

pub(super) fn validate(value: &Value) -> Result<(), MapError> {
    let bytes = serde_json::to_vec(value).map_err(|_| MapError::InvalidSnapshot)?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(MapError::SnapshotTooLarge);
    }
    let object = value.as_object().ok_or(MapError::InvalidSnapshot)?;
    let expected = [
        "state_id",
        "generation",
        "schema_version",
        "projection_version",
        "game_build",
        "mod_version",
        "map_instance_id",
        "act_id",
        "scope_id",
        "availability",
        "completeness",
        "freshness",
        "reason",
        "nodes",
        "edges",
        "position",
        "history",
        "terminal_node_ids",
        "bindings",
    ];
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(MapError::InvalidSnapshot);
    }
    if object.get("schema_version").and_then(Value::as_str) != Some(VISIBLE_MAP_SCHEMA)
        || object.get("projection_version").and_then(Value::as_str) != Some(RUNTIME_MAP_PROFILE)
        || object.get("availability").and_then(Value::as_str) != Some("available")
        || object.get("completeness").and_then(Value::as_str) != Some("complete")
        || object.get("freshness").and_then(Value::as_str) != Some("current")
        || !object.get("reason").is_some_and(Value::is_null)
    {
        return Err(MapError::SnapshotNotCurrent);
    }
    required_identity(object, "state_id", MAX_ID_BYTES)?;
    required_generation(object, "generation")?;
    required_text(object, "game_build", MAX_TEXT_BYTES)?;
    required_text(object, "mod_version", MAX_TEXT_BYTES)?;
    if nullable_identity(object, "map_instance_id")?.is_none()
        || nullable_identity(object, "scope_id")?.is_none()
    {
        return Err(MapError::InvalidIdentity);
    }
    if object
        .get("act_id")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .is_none()
    {
        return Err(MapError::InvalidIdentity);
    }
    let nodes = graph::parse_nodes(object.get("nodes"))?;
    let node_ids = nodes.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let visited_ids = object
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or(MapError::InvalidNode)?
        .iter()
        .filter(|node| node.get("visited").and_then(Value::as_bool) == Some(true))
        .filter_map(|node| node.get("id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let edges = graph::parse_edges(object.get("edges"), &node_ids)?;
    let current = graph::parse_position(object.get("position"), &node_ids, &visited_ids)?;
    let history = graph::parse_id_array(object.get("history"), &node_ids, Some(&visited_ids))?;
    if current.is_none() && !history.is_empty() {
        return Err(MapError::InvalidNode);
    }
    let _terminals = graph::parse_id_array(object.get("terminal_node_ids"), &node_ids, None)?;
    graph::validate_acyclic(&nodes, &edges)?;
    let bindings = object
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or(MapError::InvalidBinding)?;
    if bindings.len() > MAX_BINDINGS {
        return Err(MapError::InvalidBinding);
    }
    let mut graph_ids = BTreeSet::new();
    let mut host_ids = BTreeSet::new();
    let mut option_ids = BTreeSet::new();
    for binding in bindings {
        let binding = binding.as_object().ok_or(MapError::InvalidBinding)?;
        if binding.len() != 3 {
            return Err(MapError::InvalidBinding);
        }
        let graph_id = binding
            .get("graph_node_id")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidBinding)?;
        let host_id = binding
            .get("host_action_id")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidBinding)?;
        if !node_ids.contains(graph_id)
            || current.as_deref() == Some(graph_id)
            || !valid_identity(graph_id)
            || !valid_identity_limit(host_id, MAX_HOST_ACTION_ID_BYTES)
            || host_id == graph_id
            || !host_ids.insert(host_id)
            || !graph_ids.insert(graph_id)
        {
            return Err(MapError::InvalidBinding);
        }
        let action = binding
            .get("action")
            .and_then(Value::as_object)
            .ok_or(MapError::InvalidBinding)?;
        if action.len() != 2
            || action.get("kind").and_then(Value::as_str) != Some("select_map_node")
        {
            return Err(MapError::InvalidBinding);
        }
        let option_id = action
            .get("node_id")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidBinding)?;
        if !valid_identity(option_id) || !option_ids.insert(option_id) {
            return Err(MapError::InvalidBinding);
        }
    }
    Ok(())
}

fn required_identity(
    object: &Map<String, Value>,
    key: &str,
    maximum: usize,
) -> Result<String, MapError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(MapError::InvalidIdentity)?;
    if !valid_identity_limit(value, maximum) {
        return Err(MapError::InvalidIdentity);
    }
    Ok(value.to_owned())
}

fn nullable_identity(object: &Map<String, Value>, key: &str) -> Result<Option<String>, MapError> {
    let value = object.get(key).ok_or(MapError::InvalidIdentity)?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(required_identity(object, key, MAX_ID_BYTES)?))
}

fn required_generation(object: &Map<String, Value>, key: &str) -> Result<u64, MapError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_GENERATION)
        .ok_or(MapError::InvalidGeneration)
}

fn required_text(object: &Map<String, Value>, key: &str, maximum: usize) -> Result<(), MapError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(MapError::InvalidSnapshot)?;
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(MapError::InvalidSnapshot);
    }
    Ok(())
}
