// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const MAP_MAX_NODES: usize = 256;
pub const MAP_MAX_EDGES: usize = 1024;
pub const MAP_MAX_IDENTIFIER_BYTES: usize = 128;
pub const MAP_MAX_CATEGORY_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapCompleteness {
    Complete,
    Incomplete,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapNodeStatus {
    Unknown,
    Current,
    Visited,
    Available,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MapNode {
    pub node_id: String,
    pub row: i32,
    pub column: i32,
    pub category: String,
    pub status: MapNodeStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MapEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LegalDestination {
    pub node_id: String,
    pub action_id: String,
}

/// Validated harness input assembled from a protocol-owned visible snapshot.
/// This adapter is not a wire contract and carries no mutation authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidatedMapGraph {
    snapshot_digest: String,
    map_instance: String,
    act: String,
    source_state_id: String,
    generation: u64,
    completeness: MapCompleteness,
    nodes: Vec<MapNode>,
    edges: Vec<MapEdge>,
    current_node: Option<String>,
    legal_destinations: Vec<LegalDestination>,
    terminals: Vec<String>,
}

impl ValidatedMapGraph {
    pub fn new(
        snapshot_digest: impl Into<String>,
        map_instance: impl Into<String>,
        act: impl Into<String>,
        source_state_id: impl Into<String>,
        generation: u64,
        completeness: MapCompleteness,
        nodes: Vec<MapNode>,
        edges: Vec<MapEdge>,
        current_node: Option<String>,
        legal_destinations: Vec<LegalDestination>,
        terminals: Vec<String>,
    ) -> Result<Self, MapGraphError> {
        let graph = Self {
            snapshot_digest: snapshot_digest.into(),
            map_instance: map_instance.into(),
            act: act.into(),
            source_state_id: source_state_id.into(),
            generation,
            completeness,
            nodes,
            edges,
            current_node,
            legal_destinations,
            terminals,
        };
        graph.validate()?;
        Ok(graph)
    }

    fn validate(&self) -> Result<(), MapGraphError> {
        validate_digest(&self.snapshot_digest)?;
        for (name, value) in [
            ("map_instance", &self.map_instance),
            ("act", &self.act),
            ("source_state_id", &self.source_state_id),
        ] {
            validate_identifier(name, value)?;
        }
        if self.nodes.len() > MAP_MAX_NODES {
            return Err(MapGraphError::TooManyNodes(self.nodes.len()));
        }
        if self.edges.len() > MAP_MAX_EDGES {
            return Err(MapGraphError::TooManyEdges(self.edges.len()));
        }
        if matches!(self.completeness, MapCompleteness::Unavailable)
            && (!self.nodes.is_empty()
                || !self.edges.is_empty()
                || self.current_node.is_some()
                || !self.legal_destinations.is_empty()
                || !self.terminals.is_empty())
        {
            return Err(MapGraphError::UnavailableHasGraph);
        }
        let node_ids = self.validate_nodes()?;
        self.validate_edges(&node_ids)?;
        self.validate_optional_nodes(&node_ids)?;
        self.validate_legal_destinations(&node_ids)?;
        Ok(())
    }

    fn validate_nodes(&self) -> Result<BTreeSet<String>, MapGraphError> {
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            validate_identifier("node_id", &node.node_id)?;
            if node.category.is_empty() || node.category.len() > MAP_MAX_CATEGORY_BYTES {
                return Err(MapGraphError::InvalidField("category"));
            }
            if !ids.insert(node.node_id.clone()) {
                return Err(MapGraphError::DuplicateNode(node.node_id.clone()));
            }
        }
        Ok(ids)
    }

    fn validate_edges(&self, node_ids: &BTreeSet<String>) -> Result<(), MapGraphError> {
        let mut edges = BTreeSet::new();
        for edge in &self.edges {
            if !node_ids.contains(&edge.from) || !node_ids.contains(&edge.to) {
                return Err(MapGraphError::UnknownEndpoint {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                });
            }
            if !edges.insert((edge.from.clone(), edge.to.clone())) {
                return Err(MapGraphError::DuplicateEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_optional_nodes(&self, node_ids: &BTreeSet<String>) -> Result<(), MapGraphError> {
        if let Some(current) = &self.current_node {
            validate_identifier("current_node", current)?;
            if !node_ids.contains(current) {
                return Err(MapGraphError::UnknownNode(current.clone()));
            }
        }
        let mut terminals = BTreeSet::new();
        for terminal in &self.terminals {
            validate_identifier("terminal", terminal)?;
            if !node_ids.contains(terminal) {
                return Err(MapGraphError::UnknownNode(terminal.clone()));
            }
            if !terminals.insert(terminal) {
                return Err(MapGraphError::DuplicateTerminal(terminal.clone()));
            }
        }
        Ok(())
    }

    fn validate_legal_destinations(
        &self,
        node_ids: &BTreeSet<String>,
    ) -> Result<(), MapGraphError> {
        let mut destinations = BTreeSet::new();
        for destination in &self.legal_destinations {
            validate_identifier("legal destination node_id", &destination.node_id)?;
            validate_identifier("action_id", &destination.action_id)?;
            if !node_ids.contains(&destination.node_id) {
                return Err(MapGraphError::UnknownNode(destination.node_id.clone()));
            }
            if !destinations.insert(destination.node_id.clone()) {
                return Err(MapGraphError::DuplicateLegalDestination(
                    destination.node_id.clone(),
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn snapshot_digest(&self) -> &str {
        &self.snapshot_digest
    }
    #[must_use]
    pub fn map_instance(&self) -> &str {
        &self.map_instance
    }
    #[must_use]
    pub fn act(&self) -> &str {
        &self.act
    }
    #[must_use]
    pub fn source_state_id(&self) -> &str {
        &self.source_state_id
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn completeness(&self) -> &MapCompleteness {
        &self.completeness
    }
    #[must_use]
    pub fn nodes(&self) -> &[MapNode] {
        &self.nodes
    }
    #[must_use]
    pub fn edges(&self) -> &[MapEdge] {
        &self.edges
    }
    #[must_use]
    pub fn current_node(&self) -> Option<&str> {
        self.current_node.as_deref()
    }
    #[must_use]
    pub fn legal_destinations(&self) -> &[LegalDestination] {
        &self.legal_destinations
    }
    #[must_use]
    pub fn terminals(&self) -> &[String] {
        &self.terminals
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapGraphError {
    InvalidField(&'static str),
    InvalidDigest,
    TooManyNodes(usize),
    TooManyEdges(usize),
    UnavailableHasGraph,
    DuplicateNode(String),
    DuplicateEdge { from: String, to: String },
    DuplicateTerminal(String),
    DuplicateLegalDestination(String),
    UnknownNode(String),
    UnknownEndpoint { from: String, to: String },
    Wire(&'static str),
}

impl fmt::Display for MapGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid map field {field}"),
            Self::InvalidDigest => formatter.write_str("snapshot digest must be lowercase SHA-256"),
            Self::TooManyNodes(count) => write!(
                formatter,
                "map has {count} nodes; maximum is {MAP_MAX_NODES}"
            ),
            Self::TooManyEdges(count) => write!(
                formatter,
                "map has {count} edges; maximum is {MAP_MAX_EDGES}"
            ),
            Self::UnavailableHasGraph => {
                formatter.write_str("unavailable map cannot carry graph data")
            }
            Self::DuplicateNode(id) => write!(formatter, "duplicate map node {id}"),
            Self::DuplicateEdge { from, to } => {
                write!(formatter, "duplicate map edge {from}->{to}")
            }
            Self::DuplicateTerminal(id) => write!(formatter, "duplicate terminal {id}"),
            Self::DuplicateLegalDestination(id) => {
                write!(formatter, "duplicate legal destination {id}")
            }
            Self::UnknownNode(id) => write!(formatter, "unknown map node {id}"),
            Self::UnknownEndpoint { from, to } => {
                write!(formatter, "unknown edge endpoint {from}->{to}")
            }
            Self::Wire(field) => write!(formatter, "invalid visible-map wire field {field}"),
        }
    }
}

impl std::error::Error for MapGraphError {}

pub(crate) fn validate_identifier(name: &'static str, value: &str) -> Result<(), MapGraphError> {
    if value.is_empty() || value.len() > MAP_MAX_IDENTIFIER_BYTES {
        return Err(MapGraphError::InvalidField(name));
    }
    Ok(())
}

pub(crate) fn validate_digest(value: &str) -> Result<(), MapGraphError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(MapGraphError::InvalidDigest);
    }
    Ok(())
}
