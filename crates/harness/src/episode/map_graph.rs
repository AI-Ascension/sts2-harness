// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{MAX_EDGES, MAX_NODES, MapError, valid_identity};

pub(super) fn parse_nodes(value: Option<&Value>) -> Result<Vec<String>, MapError> {
    let nodes = value
        .and_then(Value::as_array)
        .ok_or(MapError::InvalidNode)?;
    if nodes.len() > MAX_NODES {
        return Err(MapError::InvalidNode);
    }
    let mut ids = BTreeSet::new();
    for node in nodes {
        let node = node.as_object().ok_or(MapError::InvalidNode)?;
        if node.len() != 5 {
            return Err(MapError::InvalidNode);
        }
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidNode)?;
        if !valid_identity(id) || !ids.insert(id) {
            return Err(MapError::InvalidNode);
        }
        if node
            .get("row")
            .and_then(Value::as_i64)
            .is_none_or(|value| !(-32_768..=32_767).contains(&value))
            || node
                .get("column")
                .and_then(Value::as_i64)
                .is_none_or(|value| !(-32_768..=32_767).contains(&value))
            || node.get("visited").and_then(Value::as_bool).is_none()
            || !matches!(
                node.get("category").and_then(Value::as_str),
                Some(
                    "unknown"
                        | "start"
                        | "monster"
                        | "elite"
                        | "rest"
                        | "shop"
                        | "event"
                        | "treasure"
                        | "boss"
                        | "other"
                )
            )
        {
            return Err(MapError::InvalidNode);
        }
    }
    Ok(ids.into_iter().map(str::to_owned).collect())
}

pub(super) fn parse_edges(
    value: Option<&Value>,
    nodes: &BTreeSet<&str>,
) -> Result<Vec<(String, String)>, MapError> {
    let edges = value
        .and_then(Value::as_array)
        .ok_or(MapError::InvalidEdge)?;
    if edges.len() > MAX_EDGES {
        return Err(MapError::InvalidEdge);
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(edges.len());
    for edge in edges {
        let edge = edge.as_object().ok_or(MapError::InvalidEdge)?;
        if edge.len() != 2 {
            return Err(MapError::InvalidEdge);
        }
        let from = edge
            .get("from")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidEdge)?;
        let to = edge
            .get("to")
            .and_then(Value::as_str)
            .ok_or(MapError::InvalidEdge)?;
        if !nodes.contains(from) || !nodes.contains(to) || from == to || !seen.insert((from, to)) {
            return Err(MapError::InvalidEdge);
        }
        result.push((from.to_owned(), to.to_owned()));
    }
    Ok(result)
}

pub(super) fn parse_position(
    value: Option<&Value>,
    nodes: &BTreeSet<&str>,
    visited: &BTreeSet<&str>,
) -> Result<Option<String>, MapError> {
    let position = value
        .and_then(Value::as_object)
        .ok_or(MapError::InvalidNode)?;
    let kind = position
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(MapError::InvalidNode)?;
    match kind {
        "pre_start" | "unavailable" if position.len() == 1 => Ok(None),
        "current" if position.len() == 2 => {
            let id = position
                .get("node_id")
                .and_then(Value::as_str)
                .ok_or(MapError::InvalidNode)?;
            if !nodes.contains(id) || !visited.contains(id) {
                return Err(MapError::InvalidNode);
            }
            Ok(Some(id.to_owned()))
        }
        _ => Err(MapError::InvalidNode),
    }
}

pub(super) fn parse_id_array(
    value: Option<&Value>,
    nodes: &BTreeSet<&str>,
    visited: Option<&BTreeSet<&str>>,
) -> Result<Vec<String>, MapError> {
    let values = value
        .and_then(Value::as_array)
        .ok_or(MapError::InvalidNode)?;
    if values.len() > MAX_NODES {
        return Err(MapError::InvalidNode);
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let id = value.as_str().ok_or(MapError::InvalidNode)?;
        if !nodes.contains(id) || !seen.insert(id) {
            return Err(MapError::InvalidNode);
        }
        if visited.is_some_and(|visited| !visited.contains(id)) {
            return Err(MapError::InvalidNode);
        }
        result.push(id.to_owned());
    }
    Ok(result)
}

pub(super) fn validate_acyclic(
    nodes: &[String],
    edges: &[(String, String)],
) -> Result<(), MapError> {
    let index = nodes
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = vec![Vec::new(); nodes.len()];
    for (from, to) in edges {
        adjacency[*index.get(from.as_str()).ok_or(MapError::InvalidEdge)?]
            .push(*index.get(to.as_str()).ok_or(MapError::InvalidEdge)?);
    }
    fn visit(index: usize, adjacency: &[Vec<usize>], state: &mut [u8]) -> bool {
        if state[index] == 1 {
            return false;
        }
        if state[index] == 2 {
            return true;
        }
        state[index] = 1;
        if adjacency[index]
            .iter()
            .all(|next| visit(*next, adjacency, state))
        {
            state[index] = 2;
            true
        } else {
            false
        }
    }
    let mut state = vec![0; nodes.len()];
    if (0..nodes.len()).all(|index| visit(index, &adjacency, &mut state)) {
        Ok(())
    } else {
        Err(MapError::InvalidEdge)
    }
}
