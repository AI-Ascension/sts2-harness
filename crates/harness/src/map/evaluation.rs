// SPDX-License-Identifier: MIT

use super::graph::MapEdge;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The four bounded public context shapes used by the synthetic map evaluator.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    NextMoveOnly,
    Graph,
    GraphAnalysis,
    GraphAnalysisImage,
}

/// Upper bounds for the synthetic evaluator. They keep the evaluator useful for deterministic
/// tests without allowing a caller to turn it into an unbounded parser or report builder.
pub const SYNTHETIC_MAX_TASKS: usize = 64;
pub const SYNTHETIC_MAX_DECISIONS: usize = 256;
pub const SYNTHETIC_MAX_NODES: usize = 256;
pub const SYNTHETIC_MAX_EDGES: usize = 1_024;
pub const SYNTHETIC_MAX_ROUTES: usize = 256;
pub const SYNTHETIC_MAX_ROUTE_NODES: usize = 256;
pub const SYNTHETIC_MAX_IDENTIFIER_BYTES: usize = 128;
pub const SYNTHETIC_MAX_REQUEST_BYTES: u32 = 256 * 1024;
pub const SYNTHETIC_MAX_LATENCY_UNITS: u32 = 120_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticGraphTask {
    pub task_id: String,
    pub nodes: Vec<String>,
    pub edges: Vec<MapEdge>,
    /// Graph node IDs that can be selected from the task's inferred root.
    pub legal_destinations: Vec<String>,
    /// Complete root-to-terminal routes that the bounded fixture considers successful.
    pub expected_routes: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticDecision {
    pub task_id: String,
    pub mode: ContextMode,
    pub proposed_route: Vec<String>,
    pub proposed_action_id: String,
    pub request_bytes: u32,
    pub latency_units: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticEvaluationRow {
    pub task_id: String,
    pub mode: ContextMode,
    pub topology_errors: u32,
    pub missed_route_opportunities: u32,
    pub invalid_proposals: u32,
    pub request_bytes: u32,
    pub latency_units: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticEvaluationReport {
    pub evaluator_version: String,
    pub rows: Vec<SyntheticEvaluationRow>,
}

pub struct SyntheticEvaluationRunner {
    tasks: Vec<SyntheticGraphTask>,
    evaluator_version: String,
}

impl SyntheticEvaluationRunner {
    pub fn new(tasks: Vec<SyntheticGraphTask>) -> Result<Self, MapEvaluationError> {
        if tasks.is_empty() || tasks.len() > SYNTHETIC_MAX_TASKS {
            return Err(MapEvaluationError::InvalidTasks);
        }
        let mut task_ids = BTreeSet::new();
        for task in &tasks {
            validate_task(task)?;
            if !task_ids.insert(task.task_id.as_str()) {
                return Err(MapEvaluationError::InvalidTasks);
            }
        }
        Ok(Self {
            tasks,
            evaluator_version: "synthetic-map-context-v2".to_owned(),
        })
    }

    pub fn evaluate(
        &self,
        decisions: &[SyntheticDecision],
    ) -> Result<SyntheticEvaluationReport, MapEvaluationError> {
        if decisions.len() > SYNTHETIC_MAX_DECISIONS {
            return Err(MapEvaluationError::TooManyDecisions);
        }
        let mut rows = Vec::with_capacity(decisions.len());
        for decision in decisions {
            validate_decision(decision)?;
            let task = self
                .tasks
                .iter()
                .find(|task| task.task_id == decision.task_id)
                .ok_or_else(|| MapEvaluationError::UnknownTask(decision.task_id.clone()))?;
            let topology = Topology::from_task(task);
            let topology_errors = count_topology_errors(&topology, &decision.proposed_route);
            let expected = task
                .expected_routes
                .iter()
                .any(|route| route == &decision.proposed_route);
            let missed_route_opportunities = u32::from(!expected);
            let invalid_proposals = u32::from(
                topology_errors > 0
                    || !is_legal_proposal(task, &decision.proposed_route)
                    || !is_canonical_action_id(
                        task,
                        &decision.proposed_route,
                        &decision.proposed_action_id,
                    ),
            );
            rows.push(SyntheticEvaluationRow {
                task_id: decision.task_id.clone(),
                mode: decision.mode,
                topology_errors,
                missed_route_opportunities,
                invalid_proposals,
                request_bytes: decision.request_bytes,
                latency_units: decision.latency_units,
            });
        }
        rows.sort_by(|left, right| {
            (left.task_id.as_str(), left.mode).cmp(&(right.task_id.as_str(), right.mode))
        });
        Ok(SyntheticEvaluationReport {
            evaluator_version: self.evaluator_version.clone(),
            rows,
        })
    }
}

/// Returns the only action identity accepted by the synthetic map task for a destination.
/// This is a deterministic test identity and has no host dispatch authority.
#[must_use]
pub fn synthetic_action_id(destination: &str) -> String {
    format!("select-map-node:{destination}")
}

#[derive(Clone, Debug)]
struct Topology {
    nodes: BTreeSet<String>,
    edges: BTreeSet<(String, String)>,
    roots: BTreeSet<String>,
    terminals: BTreeSet<String>,
}

impl Topology {
    fn from_task(task: &SyntheticGraphTask) -> Self {
        let nodes = task.nodes.iter().cloned().collect::<BTreeSet<_>>();
        let edges = task
            .edges
            .iter()
            .map(|edge| (edge.from.clone(), edge.to.clone()))
            .collect::<BTreeSet<_>>();
        let destinations = task
            .edges
            .iter()
            .map(|edge| edge.to.as_str())
            .collect::<BTreeSet<_>>();
        let sources = task
            .edges
            .iter()
            .map(|edge| edge.from.as_str())
            .collect::<BTreeSet<_>>();
        let roots = nodes
            .iter()
            .filter(|node| !destinations.contains(node.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        let terminals = nodes
            .iter()
            .filter(|node| !sources.contains(node.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        Self {
            nodes,
            edges,
            roots,
            terminals,
        }
    }
}

pub(crate) fn validate_task(task: &SyntheticGraphTask) -> Result<(), MapEvaluationError> {
    if !valid_identifier(&task.task_id)
        || task.nodes.is_empty()
        || task.nodes.len() > SYNTHETIC_MAX_NODES
        || task.edges.is_empty()
        || task.edges.len() > SYNTHETIC_MAX_EDGES
        || task.expected_routes.is_empty()
        || task.expected_routes.len() > SYNTHETIC_MAX_ROUTES
        || task.legal_destinations.is_empty()
        || task.legal_destinations.len() > SYNTHETIC_MAX_NODES
    {
        return Err(MapEvaluationError::InvalidTasks);
    }
    let nodes = task.nodes.iter().collect::<BTreeSet<_>>();
    if nodes.len() != task.nodes.len() || task.nodes.iter().any(|node| !valid_identifier(node)) {
        return Err(MapEvaluationError::InvalidTasks);
    }
    let mut edges = BTreeSet::new();
    let mut adjacency = BTreeMap::<&str, Vec<&str>>::new();
    for edge in &task.edges {
        if !nodes.contains(&edge.from)
            || !nodes.contains(&edge.to)
            || edge.from == edge.to
            || !edges.insert((&edge.from, &edge.to))
        {
            return Err(MapEvaluationError::InvalidTasks);
        }
        adjacency
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    if nodes
        .iter()
        .any(|node| has_cycle(node.as_str(), &adjacency, &mut BTreeMap::new()))
    {
        return Err(MapEvaluationError::InvalidTasks);
    }
    let topology = Topology::from_task(task);
    if topology.roots.is_empty() || topology.terminals.is_empty() {
        return Err(MapEvaluationError::InvalidTasks);
    }
    let mut legal_destinations = BTreeSet::new();
    for destination in &task.legal_destinations {
        if !valid_identifier(destination)
            || !nodes.contains(destination)
            || !legal_destinations.insert(destination)
        {
            return Err(MapEvaluationError::InvalidTasks);
        }
    }
    let mut expected_routes = BTreeSet::new();
    let mut expected_destinations = BTreeSet::new();
    for route in &task.expected_routes {
        if route.len() < 2
            || route.len() > SYNTHETIC_MAX_ROUTE_NODES
            || route
                .iter()
                .any(|node| !valid_identifier(node) || !nodes.contains(node))
            || !expected_routes.insert(route)
            || route
                .windows(2)
                .any(|pair| !edges.contains(&(&pair[0], &pair[1])) || pair[0] == pair[1])
            || route.iter().collect::<BTreeSet<_>>().len() != route.len()
            || !topology.roots.contains(&route[0])
            || !topology.terminals.contains(&route[route.len() - 1])
        {
            return Err(MapEvaluationError::InvalidTasks);
        }
        if let Some(destination) = route.get(1) {
            expected_destinations.insert(destination);
        }
    }
    if expected_destinations != legal_destinations {
        return Err(MapEvaluationError::InvalidTasks);
    }
    Ok(())
}

fn validate_decision(decision: &SyntheticDecision) -> Result<(), MapEvaluationError> {
    if !valid_identifier(&decision.task_id)
        || decision.proposed_route.len() > SYNTHETIC_MAX_ROUTE_NODES
        || decision
            .proposed_route
            .iter()
            .any(|node| node.len() > SYNTHETIC_MAX_IDENTIFIER_BYTES)
        || decision.proposed_action_id.len() > SYNTHETIC_MAX_IDENTIFIER_BYTES
        || decision.request_bytes > SYNTHETIC_MAX_REQUEST_BYTES
        || decision.latency_units > SYNTHETIC_MAX_LATENCY_UNITS
    {
        return Err(MapEvaluationError::InvalidDecision);
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SYNTHETIC_MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn has_cycle<'a>(
    node: &'a str,
    adjacency: &BTreeMap<&'a str, Vec<&'a str>>,
    colors: &mut BTreeMap<&'a str, u8>,
) -> bool {
    match colors.get(node).copied().unwrap_or(0) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    colors.insert(node, 1);
    if adjacency
        .get(node)
        .into_iter()
        .flatten()
        .any(|next| has_cycle(next, adjacency, colors))
    {
        return true;
    }
    colors.insert(node, 2);
    false
}

fn count_topology_errors(topology: &Topology, route: &[String]) -> u32 {
    if route.len() < 2 {
        return 1;
    }
    let mut errors = route
        .iter()
        .filter(|node| !topology.nodes.contains(*node))
        .count() as u32;
    if !topology.roots.contains(&route[0]) {
        errors = errors.saturating_add(1);
    }
    errors = errors.saturating_add(
        route
            .windows(2)
            .filter(|pair| !topology.edges.contains(&(pair[0].clone(), pair[1].clone())))
            .count() as u32,
    );
    if route
        .last()
        .is_none_or(|last| !topology.terminals.contains(last))
    {
        errors = errors.saturating_add(1);
    }
    errors
}

fn is_legal_proposal(task: &SyntheticGraphTask, route: &[String]) -> bool {
    route.get(1).is_some_and(|destination| {
        task.legal_destinations
            .iter()
            .any(|legal| legal == destination)
    })
}

fn is_canonical_action_id(task: &SyntheticGraphTask, route: &[String], action_id: &str) -> bool {
    route.get(1).is_some_and(|destination| {
        task.legal_destinations
            .iter()
            .any(|legal| legal == destination)
            && action_id == synthetic_action_id(destination)
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapEvaluationError {
    InvalidTasks,
    InvalidDecision,
    TooManyDecisions,
    TooLarge(&'static str),
    Serialization,
    UnknownTask(String),
}

impl fmt::Display for MapEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTasks => formatter.write_str("invalid synthetic map tasks"),
            Self::InvalidDecision => formatter.write_str("invalid synthetic map decision"),
            Self::TooManyDecisions => formatter.write_str("too many synthetic map decisions"),
            Self::TooLarge(field) => write!(formatter, "synthetic map {field} exceeds its bound"),
            Self::Serialization => {
                formatter.write_str("synthetic map context serialization failed")
            }
            Self::UnknownTask(task) => write!(formatter, "unknown synthetic map task {task}"),
        }
    }
}

impl std::error::Error for MapEvaluationError {}
