// SPDX-License-Identifier: MIT

//! Context-bound invocation discovery for the live execution coordinator.
//!
//! The admitted definition declares which nodes consume shared context. The
//! live coordinator records that set at submission and consults it at dispatch
//! so a binding is requested only for the invocation the runtime actually
//! executes.

use crate::workflow::{NodeDefinition, WorkflowDefinition};

/// One context-bound node declared by the admitted definition.
pub(super) struct ContextNode {
    pub(super) graph_id: String,
    pub(super) node_id: String,
    pub(super) node_kind: String,
    pub(super) context_ref: String,
}

/// Returns the context-bound invocations declared by an admitted definition.
///
/// Selection is by node identity, not execution order: each node is bound only
/// when the runtime cursor reaches it, so no binding is fabricated for an
/// invocation the run may never execute.
pub(super) fn context_nodes(definition: &WorkflowDefinition) -> Vec<ContextNode> {
    let mut nodes = Vec::new();
    for graph in &definition.graphs {
        for node in &graph.nodes {
            let Some((node_id, node_kind, context_ref)) = context_node_parts(node) else {
                continue;
            };
            nodes.push(ContextNode {
                graph_id: graph.id.as_str().to_owned(),
                node_id: node_id.to_owned(),
                node_kind: node_kind.to_owned(),
                context_ref: context_ref.to_owned(),
            });
        }
    }
    nodes
}

fn context_node_parts(node: &NodeDefinition) -> Option<(&str, &str, &str)> {
    match node {
        NodeDefinition::Analyze { id, config } => {
            Some((id.as_str(), "analyze", config.context_ref.as_str()))
        }
        NodeDefinition::Decide { id, config } => {
            Some((id.as_str(), "decide", config.context_ref.as_str()))
        }
        _ => None,
    }
}
