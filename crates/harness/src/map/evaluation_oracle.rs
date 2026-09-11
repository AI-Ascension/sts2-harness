// SPDX-License-Identifier: MIT

use super::bundle::MapViewBundle;
use super::evaluation::MapEvaluationError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct SnapshotGraph {
    nodes: BTreeSet<String>,
    edges: BTreeMap<String, Vec<String>>,
    current: Option<String>,
    terminals: BTreeSet<String>,
    bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Binding {
    pub(super) graph_node_id: String,
    pub(super) host_action_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BestBinding {
    Exact(Binding),
    /// At least one legal candidate's path count exceeded the checked arithmetic bound.
    /// No candidate is reported as definitive in that case.
    Unknown,
    Unavailable,
}

impl SnapshotGraph {
    pub(super) fn from_bundle(bundle: &MapViewBundle) -> Result<Self, MapEvaluationError> {
        let value: Value = serde_json::from_slice(&bundle.snapshot_bytes)
            .map_err(|_| MapEvaluationError::Serialization)?;
        let object = value.as_object().ok_or(MapEvaluationError::InvalidTasks)?;
        let nodes = object
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or(MapEvaluationError::InvalidTasks)?
            .iter()
            .filter_map(|node| node.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect::<BTreeSet<_>>();
        let mut edges = BTreeMap::<String, Vec<String>>::new();
        for edge in object
            .get("edges")
            .and_then(Value::as_array)
            .ok_or(MapEvaluationError::InvalidTasks)?
        {
            let from = edge
                .get("from")
                .and_then(Value::as_str)
                .ok_or(MapEvaluationError::InvalidTasks)?;
            let to = edge
                .get("to")
                .and_then(Value::as_str)
                .ok_or(MapEvaluationError::InvalidTasks)?;
            edges
                .entry(from.to_owned())
                .or_default()
                .push(to.to_owned());
        }
        for children in edges.values_mut() {
            children.sort();
            children.dedup();
        }
        let position = object
            .get("position")
            .ok_or(MapEvaluationError::InvalidTasks)?;
        let current = position
            .get("node_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let terminals = object
            .get("terminal_node_ids")
            .and_then(Value::as_array)
            .ok_or(MapEvaluationError::InvalidTasks)?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let bindings = object
            .get("bindings")
            .and_then(Value::as_array)
            .ok_or(MapEvaluationError::InvalidTasks)?
            .iter()
            .map(|binding| {
                Ok(Binding {
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
        Ok(Self {
            nodes,
            edges,
            current,
            terminals,
            bindings,
        })
    }

    pub(super) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn edge_count(&self) -> usize {
        self.edges.values().map(Vec::len).sum()
    }

    pub(super) fn binding_for_action(&self, action_id: &str) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|binding| binding.host_action_id == action_id)
    }

    pub(super) fn binding_errors(&self, binding: &Binding) -> u32 {
        let Some(current) = self.current.as_deref() else {
            return 1;
        };
        if !self.nodes.contains(&binding.graph_node_id) {
            return 1;
        }
        u32::from(
            !self
                .edges
                .get(current)
                .is_some_and(|children| children.binary_search(&binding.graph_node_id).is_ok()),
        )
    }

    pub(super) fn route_errors(&self, route: &[String], selected: Option<&Binding>) -> u32 {
        let Some(current) = self.current.as_deref() else {
            return 1;
        };
        let Some(first) = route.first() else {
            return 1;
        };
        let mut errors = u32::from(first != current);
        if route.len() < 2 {
            errors = errors.saturating_add(1);
        }
        for pair in route.windows(2) {
            if !self.nodes.contains(&pair[0])
                || !self.nodes.contains(&pair[1])
                || !self
                    .edges
                    .get(&pair[0])
                    .is_some_and(|children| children.binary_search(&pair[1]).is_ok())
            {
                errors = errors.saturating_add(1);
            }
        }
        if route.iter().collect::<BTreeSet<_>>().len() != route.len() {
            errors = errors.saturating_add(1);
        }
        if let (Some(binding), Some(destination)) = (selected, route.get(1))
            && binding.graph_node_id != *destination
        {
            errors = errors.saturating_add(1);
        }
        errors
    }

    pub(super) fn best_binding(&self) -> BestBinding {
        let mut candidates = Vec::new();
        for binding in self
            .bindings
            .iter()
            .filter(|binding| self.binding_errors(binding) == 0)
        {
            let count = match path_count(
                &binding.graph_node_id,
                &self.edges,
                &self.terminals,
                &mut BTreeMap::new(),
            ) {
                Ok(count) => count,
                Err(()) => return BestBinding::Unknown,
            };
            candidates.push((
                count,
                binding.graph_node_id.clone(),
                binding.host_action_id.clone(),
            ));
        }
        candidates.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        candidates.first().map_or(
            BestBinding::Unavailable,
            |(_, graph_node_id, host_action_id)| {
                BestBinding::Exact(Binding {
                    graph_node_id: graph_node_id.clone(),
                    host_action_id: host_action_id.clone(),
                })
            },
        )
    }
}

fn path_count(
    node: &str,
    edges: &BTreeMap<String, Vec<String>>,
    terminals: &BTreeSet<String>,
    memo: &mut BTreeMap<String, Result<u64, ()>>,
) -> Result<u64, ()> {
    if let Some(count) = memo.get(node) {
        return *count;
    }
    if terminals.contains(node) {
        memo.insert(node.to_owned(), Ok(1));
        return Ok(1);
    }
    let mut count = 0_u64;
    for child in edges.get(node).into_iter().flatten() {
        count = count
            .checked_add(path_count(child, edges, terminals, memo)?)
            .ok_or(())?;
    }
    memo.insert(node.to_owned(), Ok(count));
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{BestBinding, Binding, SnapshotGraph};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn path_count_overflow_is_unknown_instead_of_a_saturated_best() {
        let mut nodes = BTreeSet::from([String::from("start")]);
        let mut edges = BTreeMap::<String, Vec<String>>::new();
        let layer_count = 67_usize;
        for layer in 0..layer_count {
            let left = format!("n{layer}:left");
            let right = format!("n{layer}:right");
            nodes.insert(left.clone());
            nodes.insert(right.clone());
            let next = if layer + 1 < layer_count {
                vec![
                    format!("n{}:left", layer + 1),
                    format!("n{}:right", layer + 1),
                ]
            } else {
                Vec::new()
            };
            edges.insert(left.clone(), next.clone());
            edges.insert(right, next);
        }
        edges.insert(
            String::from("start"),
            vec![String::from("n0:left"), String::from("n0:right")],
        );
        let terminals = [
            format!("n{}:left", layer_count - 1),
            format!("n{}:right", layer_count - 1),
        ]
        .into_iter()
        .collect();
        let graph = SnapshotGraph {
            nodes,
            edges,
            current: Some(String::from("start")),
            terminals,
            bindings: vec![
                Binding {
                    graph_node_id: String::from("n0:left"),
                    host_action_id: String::from("left"),
                },
                Binding {
                    graph_node_id: String::from("n0:right"),
                    host_action_id: String::from("right"),
                },
            ],
        };
        assert_eq!(graph.best_binding(), BestBinding::Unknown);
    }
}
