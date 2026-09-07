// SPDX-License-Identifier: MIT

use super::graph::MapEdge;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    NextMoveOnly,
    Graph,
    GraphAnalysis,
    GraphAnalysisImage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticGraphTask {
    pub task_id: String,
    pub nodes: Vec<String>,
    pub edges: Vec<MapEdge>,
    pub legal_destinations: Vec<String>,
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
        if tasks.is_empty() || tasks.len() > 65_536 {
            return Err(MapEvaluationError::InvalidTasks);
        }
        for task in &tasks {
            validate_task(task)?;
        }
        Ok(Self {
            tasks,
            evaluator_version: "synthetic-map-context-v1".to_owned(),
        })
    }

    pub fn evaluate(
        &self,
        decisions: &[SyntheticDecision],
    ) -> Result<SyntheticEvaluationReport, MapEvaluationError> {
        let mut rows = Vec::with_capacity(decisions.len());
        for decision in decisions {
            let task = self
                .tasks
                .iter()
                .find(|task| task.task_id == decision.task_id)
                .ok_or_else(|| MapEvaluationError::UnknownTask(decision.task_id.clone()))?;
            let valid_nodes = task.nodes.iter().collect::<BTreeSet<_>>();
            let edges = task
                .edges
                .iter()
                .map(|edge| (&edge.from, &edge.to))
                .collect::<BTreeSet<_>>();
            let mut topology_errors = 0;
            for pair in decision.proposed_route.windows(2) {
                let from = valid_nodes.contains(&pair[0]);
                let to = valid_nodes.contains(&pair[1]);
                if !from || !to || !edges.contains(&(&pair[0], &pair[1])) {
                    topology_errors += 1;
                }
            }
            let expected = task
                .expected_routes
                .iter()
                .any(|route| route == &decision.proposed_route);
            let missed_route_opportunities = u32::from(!expected);
            let first_destination = decision.proposed_route.get(1);
            let valid_action = first_destination.is_some_and(|destination| {
                task.legal_destinations
                    .iter()
                    .any(|node| node == destination)
            });
            let invalid_proposals =
                u32::from(!valid_action || decision.proposed_action_id.is_empty());
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

fn validate_task(task: &SyntheticGraphTask) -> Result<(), MapEvaluationError> {
    if task.task_id.is_empty() || task.task_id.len() > 128 || task.nodes.is_empty() {
        return Err(MapEvaluationError::InvalidTasks);
    }
    let nodes = task.nodes.iter().collect::<BTreeSet<_>>();
    if nodes.len() != task.nodes.len()
        || task.edges.iter().any(|edge| {
            !nodes.contains(&edge.from) || !nodes.contains(&edge.to) || edge.from == edge.to
        })
        || task
            .legal_destinations
            .iter()
            .any(|node| !nodes.contains(node))
        || task
            .expected_routes
            .iter()
            .any(|route| route.is_empty() || route.iter().any(|node| !nodes.contains(node)))
    {
        return Err(MapEvaluationError::InvalidTasks);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapEvaluationError {
    InvalidTasks,
    UnknownTask(String),
}

impl fmt::Display for MapEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTasks => formatter.write_str("invalid synthetic map tasks"),
            Self::UnknownTask(task) => write!(formatter, "unknown synthetic map task {task}"),
        }
    }
}

impl std::error::Error for MapEvaluationError {}
