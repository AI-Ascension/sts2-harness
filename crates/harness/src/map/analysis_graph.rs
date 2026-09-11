// SPDX-License-Identifier: MIT

use super::graph::ValidatedMapGraph;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(crate) fn adjacency(
    graph: &ValidatedMapGraph,
) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
    let mut forward = graph
        .nodes()
        .iter()
        .map(|node| (node.node_id.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut reverse = forward
        .keys()
        .map(|id| (id.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in graph.edges() {
        if let Some(next) = forward.get_mut(&edge.from) {
            next.push(edge.to.clone());
        }
        if let Some(previous) = reverse.get_mut(&edge.to) {
            previous.push(edge.from.clone());
        }
    }
    for values in forward.values_mut().chain(reverse.values_mut()) {
        values.sort();
    }
    (forward, reverse)
}

pub(crate) fn topological_order(
    graph: &ValidatedMapGraph,
    adjacency: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<String>, super::analysis::MapAnalysisError> {
    let mut indegree = graph
        .nodes()
        .iter()
        .map(|node| (node.node_id.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    for next in adjacency.values() {
        for node in next {
            if let Some(value) = indegree.get_mut(node) {
                *value += 1;
            }
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(node.clone()))
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(graph.nodes().len());
    while let Some(node) = ready.pop_first() {
        order.push(node.clone());
        if let Some(next) = adjacency.get(&node) {
            for child in next {
                if let Some(value) = indegree.get_mut(child) {
                    *value -= 1;
                    if *value == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
    }
    if order.len() != graph.nodes().len() {
        let cycle_nodes = indegree
            .into_iter()
            .filter_map(|(node, degree)| (degree > 0).then_some(node))
            .collect();
        return Err(super::analysis::MapAnalysisError::Cycle(cycle_nodes));
    }
    Ok(order)
}

pub(crate) fn collect_reachable(
    start: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![start.to_owned()];
    while let Some(node) = pending.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        if let Some(next) = adjacency.get(&node) {
            pending.extend(next.iter().cloned());
        }
    }
    seen.remove(start);
    seen
}

pub(crate) fn distances(
    start: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, u32> {
    let mut values: BTreeMap<String, u32> = BTreeMap::from([(start.to_owned(), 0)]);
    let mut pending = VecDeque::from([start.to_owned()]);
    while let Some(node) = pending.pop_front() {
        let Some(distance) = values.get(&node).copied() else {
            continue;
        };
        if let Some(next) = adjacency.get(&node) {
            for child in next {
                if !values.contains_key(child) {
                    values.insert(child.clone(), distance.saturating_add(1));
                    pending.push_back(child.clone());
                }
            }
        }
    }
    values
}

pub(crate) fn terminal_distances(
    graph: &ValidatedMapGraph,
    adjacency: &BTreeMap<String, Vec<String>>,
    order: &[String],
) -> BTreeMap<String, u32> {
    let terminals = graph.terminals().iter().collect::<BTreeSet<_>>();
    let mut values: BTreeMap<String, u32> = BTreeMap::new();
    for node in order.iter().rev() {
        if terminals.contains(node) {
            values.insert(node.clone(), 0);
            continue;
        }
        let best = adjacency
            .get(node)
            .into_iter()
            .flatten()
            .filter_map(|child| values.get(child).copied())
            .min()
            .map(|distance| distance.saturating_add(1));
        if let Some(distance) = best {
            values.insert(node.clone(), distance);
        }
    }
    values
}

pub(crate) fn category_distances(
    start: &str,
    graph: &ValidatedMapGraph,
    adjacency: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, u32> {
    let nodes = distances(start, adjacency);
    let mut values: BTreeMap<String, u32> = BTreeMap::new();
    for node in graph.nodes() {
        if let Some(distance) = nodes.get(&node.node_id) {
            values
                .entry(node.category.clone())
                .and_modify(|current: &mut u32| *current = (*current).min(*distance))
                .or_insert(*distance);
        }
    }
    values
}
