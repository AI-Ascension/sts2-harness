// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use super::definition::{
    GraphDefinition, GuardDefinition, GuardExpression, GuardValue, NodeDefinition, NodeKind,
    WorkflowMode,
};
use super::diagnostic::{Diagnostic, DiagnosticCode};
use super::graph_checks::{validate_acyclic, validate_loop_graphs, validate_reachability};
use super::ids::{GraphId, NodeId};
use super::validation::{push, push_at};
use super::values::ValueType;

const MAX_NODES_PER_GRAPH: usize = 256;
const MAX_EDGES_PER_GRAPH: usize = 1024;
const MAX_GUARDS: usize = 128;
const MAX_GUARD_NODES: usize = 256;

pub(super) fn validate_graph(
    mode: WorkflowMode,
    graph: &GraphDefinition,
    graph_indexes: &BTreeMap<GraphId, usize>,
    graph_index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if graph.nodes.is_empty() || graph.nodes.len() > MAX_NODES_PER_GRAPH {
        push_at(
            diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.nodes",
            graph_index,
        );
    }
    if graph.edges.len() > MAX_EDGES_PER_GRAPH {
        push_at(
            diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.edges",
            graph_index,
        );
    }
    if graph.guards.len() > MAX_GUARDS {
        push_at(
            diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.guards",
            graph_index,
        );
    }

    let mut nodes = BTreeMap::new();
    for (node_index, node) in graph.nodes.iter().enumerate() {
        if nodes.insert(node.id().clone(), node_index).is_some() {
            push_at(
                diagnostics,
                DiagnosticCode::DuplicateIdentifier,
                "$.graphs.nodes",
                node_index,
            );
        }
        if mode == WorkflowMode::Strict && node.kind() == NodeKind::AdaptiveRegion {
            push_at(
                diagnostics,
                DiagnosticCode::UnsupportedNode,
                "$.graphs.nodes",
                node_index,
            );
        }
        validate_node(node, graph, graph_index, node_index, diagnostics);
    }
    if !nodes.contains_key(&graph.entry_node) {
        push_at(
            diagnostics,
            DiagnosticCode::MissingReference,
            "$.graphs.entry_node",
            graph_index,
        );
    }

    let mut guards = BTreeSet::new();
    for (guard_index, guard) in graph.guards.iter().enumerate() {
        if !guards.insert(guard.id.clone()) {
            push_at(
                diagnostics,
                DiagnosticCode::DuplicateIdentifier,
                "$.graphs.guards",
                guard_index,
            );
        }
        validate_guard(guard, graph_index, guard_index, diagnostics);
    }

    let mut adjacency: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut reverse: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut route_keys = BTreeSet::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        if !nodes.contains_key(&edge.from) || !nodes.contains_key(&edge.to) {
            push_at(
                diagnostics,
                DiagnosticCode::MissingReference,
                "$.graphs.edges",
                edge_index,
            );
        }
        if edge.priority > 1024 {
            push_at(
                diagnostics,
                DiagnosticCode::InvalidLimit,
                "$.graphs.edges",
                edge_index,
            );
        }
        if !route_keys.insert((edge.from.clone(), edge.on, edge.priority)) {
            push_at(
                diagnostics,
                DiagnosticCode::PriorityTie,
                "$.graphs.edges",
                edge_index,
            );
        }
        if let Some(guard_ref) = &edge.guard_ref
            && !guards.contains(guard_ref)
        {
            push_at(
                diagnostics,
                DiagnosticCode::MissingReference,
                "$.graphs.edges",
                edge_index,
            );
        }
        if nodes
            .get(&edge.from)
            .and_then(|index| graph.nodes.get(*index))
            .is_some_and(|node| node.kind() == NodeKind::Terminal)
        {
            push_at(
                diagnostics,
                DiagnosticCode::UnsupportedNode,
                "$.graphs.edges",
                edge_index,
            );
        }
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
        reverse
            .entry(edge.to.clone())
            .or_default()
            .push(edge.from.clone());
    }
    validate_reachability(graph, &nodes, &adjacency, &reverse, diagnostics);
    validate_acyclic(&nodes, &adjacency, diagnostics);
    validate_loop_graphs(graph, graph_indexes, &guards, diagnostics);
}

fn validate_node(
    node: &NodeDefinition,
    graph: &GraphDefinition,
    graph_index: usize,
    node_index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let path = format!("$.graphs[{graph_index}].nodes[{node_index}]");
    match node {
        NodeDefinition::AdaptiveRegion { config, .. } => {
            if config.allowed_operations.is_empty()
                || config.allowed_operations.len() > 32
                || config.max_plan_nodes == 0
                || config.max_plan_nodes > 32
                || config.max_plan_edges > 128
                || config.max_replans > 8
            {
                push(diagnostics, DiagnosticCode::InvalidLimit, &path);
            }
            let mut operations = BTreeSet::new();
            for operation in &config.allowed_operations {
                if !operations.insert(operation) {
                    push(diagnostics, DiagnosticCode::DuplicateIdentifier, &path);
                }
            }
        }
        NodeDefinition::ExecuteAction { config, .. } => validate_binding(
            &config.proposal_from,
            graph,
            path.as_str(),
            diagnostics,
            true,
        ),
        NodeDefinition::EmitArtifact { config, .. } => {
            validate_binding(&config.input_from, graph, path.as_str(), diagnostics, false)
        }
        NodeDefinition::Loop { config, .. }
            if !(1..=4096).contains(&config.max_iterations)
                || !graph
                    .guards
                    .iter()
                    .any(|guard| guard.id == config.exit_guard_ref) =>
        {
            push(diagnostics, DiagnosticCode::MissingReference, &path);
        }
        _ => {}
    }
    if let NodeDefinition::AwaitStability { config, .. } = node
        && !(1..=3_600_000).contains(&config.deadline_ms)
    {
        push(diagnostics, DiagnosticCode::InvalidLimit, &path);
    }
}

fn validate_binding(
    binding: &super::definition::ProposalBinding,
    graph: &GraphDefinition,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
    require_proposal: bool,
) {
    let Some(source) = graph
        .nodes
        .iter()
        .find(|candidate| candidate.id() == &binding.node_id)
    else {
        push(diagnostics, DiagnosticCode::MissingReference, path);
        return;
    };
    let output_type = source.output_type(binding.output.as_str());
    if output_type.is_none() {
        push(diagnostics, DiagnosticCode::MissingReference, path);
    } else if require_proposal && output_type != Some(ValueType::DecisionProposal) {
        push(diagnostics, DiagnosticCode::TypeMismatch, path);
    }
}

fn validate_guard(
    guard: &GuardDefinition,
    graph_index: usize,
    guard_index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (nodes, depth) = guard_size(&guard.expression);
    if nodes > MAX_GUARD_NODES || depth > 32 {
        push_at(
            diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.guards",
            graph_index + guard_index,
        );
    }
    if !guard_values_bounded(&guard.expression) {
        push_at(
            diagnostics,
            DiagnosticCode::InvalidLimit,
            "$.graphs.guards",
            guard_index,
        );
    }
}

fn guard_size(expression: &GuardExpression) -> (usize, usize) {
    match expression {
        GuardExpression::And(values) | GuardExpression::Or(values) => {
            values.iter().fold((1, 1), |(count, depth), value| {
                let (child_count, child_depth) = guard_size(value);
                (count + child_count, depth.max(child_depth + 1))
            })
        }
        GuardExpression::Not(value) => {
            let (count, depth) = guard_size(value);
            (count + 1, depth + 1)
        }
        _ => (1, 1),
    }
}

fn guard_values_bounded(expression: &GuardExpression) -> bool {
    match expression {
        GuardExpression::And(values) | GuardExpression::Or(values) => {
            !values.is_empty() && values.iter().all(guard_values_bounded)
        }
        GuardExpression::Not(value) => guard_values_bounded(value),
        GuardExpression::In { values, .. } => {
            !values.is_empty() && values.len() <= 64 && values.iter().all(value_bounded)
        }
        GuardExpression::Equal { right, .. }
        | GuardExpression::NotEqual { right, .. }
        | GuardExpression::Less { right, .. }
        | GuardExpression::LessOrEqual { right, .. }
        | GuardExpression::Greater { right, .. }
        | GuardExpression::GreaterOrEqual { right, .. } => value_bounded(right),
        GuardExpression::Literal(value) => value_bounded(value),
        GuardExpression::Field(_) | GuardExpression::Exists(_) | GuardExpression::Add { .. } => {
            true
        }
    }
}

fn value_bounded(value: &GuardValue) -> bool {
    match value {
        GuardValue::Text(text) => !text.is_empty() && text.len() <= super::ids::MAX_STRING_BYTES,
        GuardValue::Null | GuardValue::Boolean(_) | GuardValue::Integer(_) => true,
    }
}

pub(super) use super::graph_checks::validate_graph_dependencies;
