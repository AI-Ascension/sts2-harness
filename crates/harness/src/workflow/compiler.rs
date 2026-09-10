// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::canonical::CanonicalError;
use super::definition::{
    ControlEdge, GraphDefinition, GuardExpression, NodeDefinition, WorkflowDefinition,
};
use super::ids::{Digest, GraphId, GuardId, NodeId};
use super::validation::{ValidationError, validate_definition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    Validation(ValidationError),
    Canonical(CanonicalError),
    DuplicateNode(NodeId),
    MissingGraph(GraphId),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => write!(formatter, "workflow validation failed: {error}"),
            Self::Canonical(error) => {
                write!(formatter, "workflow canonicalization failed: {error}")
            }
            Self::DuplicateNode(node) => write!(formatter, "workflow node is duplicated: {node}"),
            Self::MissingGraph(graph) => write!(formatter, "workflow graph is missing: {graph}"),
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledGraph {
    id: GraphId,
    entry_node: NodeId,
    nodes: BTreeMap<NodeId, NodeDefinition>,
    edges: BTreeMap<NodeId, Vec<ControlEdge>>,
    guards: BTreeMap<GuardId, GuardExpression>,
}

impl CompiledGraph {
    #[must_use]
    pub fn id(&self) -> &GraphId {
        &self.id
    }

    #[must_use]
    pub fn entry_node(&self) -> &NodeId {
        &self.entry_node
    }

    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&NodeDefinition> {
        self.nodes.get(id)
    }

    #[must_use]
    pub fn edges_from(&self, id: &NodeId) -> &[ControlEdge] {
        self.edges.get(id).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn guard(&self, id: &GuardId) -> Option<&GuardExpression> {
        self.guards.get(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledWorkflow {
    definition: WorkflowDefinition,
    semantic_digest: Digest,
    graphs: BTreeMap<GraphId, CompiledGraph>,
}

impl CompiledWorkflow {
    pub fn compile(definition: WorkflowDefinition) -> Result<Self, CompileError> {
        validate_definition(&definition).map_err(CompileError::Validation)?;
        let semantic_digest = definition
            .semantic_digest()
            .map_err(CompileError::Canonical)?;
        let mut graphs = BTreeMap::new();
        for graph in &definition.graphs {
            let compiled = compile_graph(graph)?;
            graphs.insert(graph.id.clone(), compiled);
        }
        Ok(Self {
            definition,
            semantic_digest,
            graphs,
        })
    }

    #[must_use]
    pub fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }

    #[must_use]
    pub fn semantic_digest(&self) -> &Digest {
        &self.semantic_digest
    }

    #[must_use]
    pub fn graph(&self, id: &GraphId) -> Option<&CompiledGraph> {
        self.graphs.get(id)
    }

    pub fn entry_graph(&self) -> Result<&CompiledGraph, CompileError> {
        self.graph(&self.definition.entry_graph)
            .ok_or_else(|| CompileError::MissingGraph(self.definition.entry_graph.clone()))
    }
}

fn compile_graph(graph: &GraphDefinition) -> Result<CompiledGraph, CompileError> {
    let mut nodes = BTreeMap::new();
    for node in &graph.nodes {
        if nodes.insert(node.id().clone(), node.clone()).is_some() {
            return Err(CompileError::DuplicateNode(node.id().clone()));
        }
    }
    let mut edges: BTreeMap<NodeId, Vec<ControlEdge>> = BTreeMap::new();
    for edge in &graph.edges {
        edges
            .entry(edge.from.clone())
            .or_default()
            .push(edge.clone());
    }
    for candidates in edges.values_mut() {
        candidates.sort_by_key(|edge| (edge.priority, edge.to.clone(), edge.on));
    }
    let guards = graph
        .guards
        .iter()
        .map(|guard| (guard.id.clone(), guard.expression.clone()))
        .collect();
    Ok(CompiledGraph {
        id: graph.id.clone(),
        entry_node: graph.entry_node.clone(),
        nodes,
        edges,
        guards,
    })
}
