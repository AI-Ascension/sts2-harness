// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::definition::{GraphDefinition, NodeDefinition, NodeKind};
use super::diagnostic::{Diagnostic, DiagnosticCode};
use super::ids::{GraphId, NodeId};
use super::validation::push;

pub(super) fn validate_reachability(
    graph: &GraphDefinition,
    nodes: &BTreeMap<NodeId, usize>,
    adjacency: &BTreeMap<NodeId, Vec<NodeId>>,
    reverse: &BTreeMap<NodeId, Vec<NodeId>>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut reachable = BTreeSet::new();
    let mut queue = VecDeque::new();
    if nodes.contains_key(&graph.entry_node) {
        reachable.insert(graph.entry_node.clone());
        queue.push_back(graph.entry_node.clone());
    }
    while let Some(node) = queue.pop_front() {
        for child in adjacency.get(&node).into_iter().flatten() {
            if reachable.insert(child.clone()) {
                queue.push_back(child.clone());
            }
        }
    }
    for node in nodes.keys() {
        if !reachable.contains(node) {
            push(
                diagnostics,
                DiagnosticCode::UnreachableTerminal,
                "$.graphs.nodes",
            );
        }
    }

    let terminals: BTreeSet<NodeId> = graph
        .nodes
        .iter()
        .filter(|node| node.kind() == NodeKind::Terminal)
        .map(|node| node.id().clone())
        .collect();
    if terminals.is_empty() {
        push(
            diagnostics,
            DiagnosticCode::UnreachableTerminal,
            "$.graphs.nodes",
        );
        return;
    }
    let mut can_finish = terminals.clone();
    let mut queue: VecDeque<NodeId> = terminals.into_iter().collect();
    while let Some(node) = queue.pop_front() {
        for parent in reverse.get(&node).into_iter().flatten() {
            if can_finish.insert(parent.clone()) {
                queue.push_back(parent.clone());
            }
        }
    }
    for node in nodes.keys() {
        if !can_finish.contains(node) {
            push(
                diagnostics,
                DiagnosticCode::UnreachableTerminal,
                "$.graphs.nodes",
            );
        }
    }
}

pub(super) fn validate_acyclic(
    nodes: &BTreeMap<NodeId, usize>,
    adjacency: &BTreeMap<NodeId, Vec<NodeId>>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut incoming: BTreeMap<NodeId, usize> =
        nodes.keys().cloned().map(|node| (node, 0)).collect();
    for child in adjacency.values().flatten() {
        if let Some(count) = incoming.get_mut(child) {
            *count = count.saturating_add(1);
        }
    }
    let mut queue: VecDeque<NodeId> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| node.clone())
        .collect();
    let mut processed = 0usize;
    while let Some(node) = queue.pop_front() {
        processed += 1;
        for child in adjacency.get(&node).into_iter().flatten() {
            if let Some(count) = incoming.get_mut(child) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    queue.push_back(child.clone());
                }
            }
        }
    }
    if processed != nodes.len() {
        push(diagnostics, DiagnosticCode::GraphCycle, "$.graphs.edges");
    }
}

pub(super) fn validate_loop_graphs(
    graph: &GraphDefinition,
    graph_indexes: &BTreeMap<GraphId, usize>,
    guards: &BTreeSet<super::ids::GuardId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for node in &graph.nodes {
        if let NodeDefinition::Loop { config, .. } = node
            && !graph_indexes.contains_key(&config.body_graph)
        {
            push(
                diagnostics,
                DiagnosticCode::MissingReference,
                "$.graphs.nodes.config.body_graph",
            );
        }
        if let NodeDefinition::Loop { config, .. } = node
            && !guards.contains(&config.exit_guard_ref)
        {
            push(
                diagnostics,
                DiagnosticCode::MissingReference,
                "$.graphs.nodes.config.exit_guard_ref",
            );
        }
    }
}

pub(super) fn validate_graph_dependencies(
    graphs: &[GraphDefinition],
    graph_indexes: &BTreeMap<GraphId, usize>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut dependencies: BTreeMap<GraphId, Vec<GraphId>> = BTreeMap::new();
    for graph in graphs {
        let refs = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                NodeDefinition::Loop { config, .. }
                    if graph_indexes.contains_key(&config.body_graph) =>
                {
                    Some(config.body_graph.clone())
                }
                _ => None,
            })
            .collect();
        dependencies.insert(graph.id.clone(), refs);
    }
    let mut state = BTreeMap::new();
    for graph in graphs {
        if graph_cycle(&graph.id, &dependencies, &mut state) {
            push(
                diagnostics,
                DiagnosticCode::GraphCycle,
                "$.graphs.nodes.config.body_graph",
            );
        }
    }
}

fn graph_cycle(
    graph: &GraphId,
    dependencies: &BTreeMap<GraphId, Vec<GraphId>>,
    state: &mut BTreeMap<GraphId, u8>,
) -> bool {
    match state.get(graph).copied() {
        Some(1) => return true,
        Some(2) => return false,
        _ => {}
    }
    state.insert(graph.clone(), 1);
    if let Some(children) = dependencies.get(graph) {
        for child in children {
            if graph_cycle(child, dependencies, state) {
                return true;
            }
        }
    }
    state.insert(graph.clone(), 2);
    false
}
