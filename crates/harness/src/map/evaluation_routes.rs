// SPDX-License-Identifier: MIT

use super::evaluation::{MapEvaluationError, SYNTHETIC_MAX_ROUTE_NODES};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn parse_object(bytes: &[u8]) -> Result<Value, MapEvaluationError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| MapEvaluationError::Serialization)?;
    value
        .is_object()
        .then_some(value)
        .ok_or(MapEvaluationError::InvalidTasks)
}

pub(super) fn terminal_route(
    node: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    terminals: &BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    memo: &mut BTreeMap<String, Option<Vec<String>>>,
) -> Option<Vec<String>> {
    if let Some(route) = memo.get(node) {
        return route.clone();
    }
    if visited.len() >= SYNTHETIC_MAX_ROUTE_NODES.saturating_sub(1) {
        return None;
    }
    if !visited.insert(node.to_owned()) {
        return None;
    }
    let route = if terminals.contains(node) {
        Some(vec![node.to_owned()])
    } else {
        let mut best = adjacency
            .get(node)
            .into_iter()
            .flatten()
            .filter_map(|child| {
                terminal_route(child, adjacency, terminals, visited, memo).map(|suffix| {
                    let mut route = Vec::with_capacity(suffix.len() + 1);
                    route.push(node.to_owned());
                    route.extend(suffix);
                    route
                })
            })
            .collect::<Vec<_>>();
        best.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
        best.into_iter().next()
    };
    visited.remove(node);
    memo.insert(node.to_owned(), route.clone());
    route
}

/// Applies one deterministic adapter to the context actually delivered to the model.
///
/// A context without a graph can establish only the current node and one known hop. When the
/// graph is present, candidate bindings are checked against its adjacency and each candidate is
/// followed to a terminal. This keeps the adapter independent of the matrix mode and avoids
/// manufacturing route nodes that were not delivered in the context.
pub(super) fn proposed_decision(
    context_bytes: &[u8],
) -> Result<(String, Vec<String>), MapEvaluationError> {
    let context = parse_object(context_bytes)?;
    let graph_context = context.get("graph").is_some();
    let graph = context.get("graph").unwrap_or(&context);
    let position = graph
        .get("position")
        .and_then(|value| value.get("node_id"))
        .and_then(Value::as_str)
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let bindings = graph
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let mut candidates = bindings
        .iter()
        .map(|binding| {
            Ok(BindingCandidate {
                graph_node_id: binding
                    .get("graph_node_id")
                    .and_then(Value::as_str)
                    .ok_or(MapEvaluationError::InvalidTasks)?
                    .to_owned(),
                host_action_id: binding
                    .get("host_action_id")
                    .and_then(Value::as_str)
                    .ok_or(MapEvaluationError::InvalidTasks)?
                    .to_owned(),
            })
        })
        .collect::<Result<Vec<_>, MapEvaluationError>>()?;
    candidates.sort_by(|left, right| {
        left.graph_node_id
            .cmp(&right.graph_node_id)
            .then_with(|| left.host_action_id.cmp(&right.host_action_id))
    });

    let Some(edges_value) = graph.get("edges") else {
        if graph_context {
            return Err(MapEvaluationError::InvalidTasks);
        }
        let candidate = candidates.first().ok_or(MapEvaluationError::InvalidTasks)?;
        return Ok((
            candidate.host_action_id.clone(),
            vec![position.to_owned(), candidate.graph_node_id.clone()],
        ));
    };
    let edges = edges_value
        .as_array()
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let mut adjacency = BTreeMap::<String, Vec<String>>::new();
    for edge in edges {
        let from = edge
            .get("from")
            .and_then(Value::as_str)
            .ok_or(MapEvaluationError::InvalidTasks)?;
        let to = edge
            .get("to")
            .and_then(Value::as_str)
            .ok_or(MapEvaluationError::InvalidTasks)?;
        adjacency
            .entry(from.to_owned())
            .or_default()
            .push(to.to_owned());
    }
    for children in adjacency.values_mut() {
        children.sort();
        children.dedup();
    }
    let terminals = graph
        .get("terminal_node_ids")
        .and_then(Value::as_array)
        .ok_or(MapEvaluationError::InvalidTasks)?
        .iter()
        .map(|terminal| {
            terminal
                .as_str()
                .map(str::to_owned)
                .ok_or(MapEvaluationError::InvalidTasks)
        })
        .collect::<Result<BTreeSet<_>, MapEvaluationError>>()?;

    let mut route_memo = BTreeMap::new();
    let mut routes = candidates
        .iter()
        .filter(|candidate| {
            adjacency
                .get(position)
                .is_some_and(|children| children.binary_search(&candidate.graph_node_id).is_ok())
        })
        .filter_map(|candidate| {
            let mut visited = BTreeSet::new();
            terminal_route(
                &candidate.graph_node_id,
                &adjacency,
                &terminals,
                &mut visited,
                &mut route_memo,
            )
            .map(|suffix| {
                let mut route = Vec::with_capacity(suffix.len() + 1);
                route.push(position.to_owned());
                route.extend(suffix);
                (route, candidate.host_action_id.clone())
            })
        })
        .collect::<Vec<_>>();
    routes.sort_by(|left, right| {
        left.0
            .len()
            .cmp(&right.0.len())
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.1.cmp(&right.1))
    });
    if let Some((route, action)) = routes.into_iter().next() {
        return Ok((action, route));
    }

    // The graph may be incomplete or no candidate may reach a declared terminal. Preserve the
    // one hop that was actually delivered so the scorer reports the missing continuation.
    let candidate = candidates.first().ok_or(MapEvaluationError::InvalidTasks)?;
    Ok((
        candidate.host_action_id.clone(),
        vec![position.to_owned(), candidate.graph_node_id.clone()],
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BindingCandidate {
    graph_node_id: String,
    host_action_id: String,
}
