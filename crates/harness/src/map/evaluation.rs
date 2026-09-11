// SPDX-License-Identifier: MIT

use super::bundle::{MapBundleError, MapViewBundle, RuntimeMapBundleIdentity};
use super::evaluation_oracle::{BestBinding, SnapshotGraph};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

/// The public context variants used by the offline evaluation matrix.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    NextMoveOnly,
    Graph,
    GraphAnalysis,
    GraphAnalysisImage,
}

pub const SYNTHETIC_MAX_TASKS: usize = 64;
pub const SYNTHETIC_MAX_DECISIONS: usize = 256;
pub const SYNTHETIC_MAX_IDENTIFIER_BYTES: usize = 128;
pub const SYNTHETIC_MAX_ACTION_ID_BYTES: usize = 512;
pub const SYNTHETIC_MAX_NODES: usize = 256;
pub const SYNTHETIC_MAX_EDGES: usize = 1_024;
pub const SYNTHETIC_MAX_ROUTES: usize = 256;
pub const SYNTHETIC_MAX_REQUEST_BYTES: u32 = 256 * 1024;
pub const SYNTHETIC_MAX_ROUTE_NODES: usize = 256;
pub const SYNTHETIC_MAX_LATENCY_MICROS: u64 = 120_000_000;

fn all_context_modes() -> [ContextMode; 4] {
    [
        ContextMode::NextMoveOnly,
        ContextMode::Graph,
        ContextMode::GraphAnalysis,
        ContextMode::GraphAnalysisImage,
    ]
}

/// One admitted public snapshot. The evaluator derives the graph and analysis from these bytes;
/// it does not accept an expected route supplied by a fixture author.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticGraphTask {
    pub task_id: String,
    pub snapshot: Vec<u8>,
}

impl SyntheticGraphTask {
    pub fn new(task_id: impl Into<String>, snapshot: Vec<u8>) -> Result<Self, MapEvaluationError> {
        let task = Self {
            task_id: task_id.into(),
            snapshot,
        };
        valid_identifier(&task.task_id)
            .then_some(task)
            .ok_or(MapEvaluationError::InvalidTasks)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticDecision {
    pub task_id: String,
    pub mode: ContextMode,
    pub proposed_route: Vec<String>,
    pub proposed_action_id: String,
    pub request_bytes: u32,
    pub graph_bytes: u32,
    pub analysis_bytes: u32,
    pub image_bytes: u32,
    pub image_status: String,
    pub latency_micros: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticEvaluationRow {
    pub task_id: String,
    pub mode: ContextMode,
    pub topology_errors: u32,
    pub missed_route_opportunities: u32,
    pub invalid_proposals: u32,
    pub request_bytes: u32,
    pub graph_bytes: u32,
    pub analysis_bytes: u32,
    pub image_bytes: u32,
    pub image_status: String,
    pub latency_micros: u64,
    pub node_count: u32,
    pub edge_count: u32,
    pub analysis_digest: String,
    pub snapshot_digest: String,
    pub decision_action_id: String,
    pub independent_best_action_id: Option<String>,
    pub independent_best_status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntheticEvaluationReport {
    pub evaluator_version: String,
    pub rows: Vec<SyntheticEvaluationRow>,
    /// Whether the report passed the four-mode acceptance contract. The low-level `evaluate`
    /// scorer deliberately leaves this false; use `evaluate_complete` for an acceptance result.
    pub complete: bool,
    pub incomplete_reasons: Vec<String>,
}

pub struct SyntheticEvaluationRunner {
    bundles: BTreeMap<String, MapViewBundle>,
    evaluator_version: String,
}

impl SyntheticEvaluationRunner {
    pub fn new(tasks: Vec<SyntheticGraphTask>) -> Result<Self, MapEvaluationError> {
        if tasks.is_empty() || tasks.len() > SYNTHETIC_MAX_TASKS {
            return Err(MapEvaluationError::InvalidTasks);
        }
        let mut bundles = BTreeMap::new();
        for task in tasks {
            if bundles.contains_key(&task.task_id)
                || task.snapshot.len() > SYNTHETIC_MAX_REQUEST_BYTES as usize
            {
                return Err(MapEvaluationError::InvalidTasks);
            }
            let bundle = bundle_for_task(&task)?;
            bundles.insert(task.task_id, bundle);
        }
        Ok(Self {
            bundles,
            evaluator_version: "synthetic-map-context-v3".to_owned(),
        })
    }

    pub(crate) fn bundle(&self, task_id: &str) -> Result<&MapViewBundle, MapEvaluationError> {
        self.bundles
            .get(task_id)
            .ok_or_else(|| MapEvaluationError::UnknownTask(task_id.to_owned()))
    }

    /// Scores the supplied rows without requiring one unique row for every task and context mode.
    /// This explicitly partial scorer is useful for focused diagnostics; acceptance callers must
    /// use [`Self::evaluate_complete`] so missing and duplicate rows cannot be mistaken for a
    /// full experiment.
    pub fn score_partial(
        &self,
        decisions: &[SyntheticDecision],
    ) -> Result<SyntheticEvaluationReport, MapEvaluationError> {
        if decisions.len() > SYNTHETIC_MAX_DECISIONS {
            return Err(MapEvaluationError::TooManyDecisions);
        }
        let mut rows = decisions
            .iter()
            .map(|decision| self.evaluate_one(decision))
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort_by(|left, right| {
            (left.task_id.as_str(), left.mode).cmp(&(right.task_id.as_str(), right.mode))
        });
        Ok(SyntheticEvaluationReport {
            evaluator_version: self.evaluator_version.clone(),
            rows,
            complete: false,
            incomplete_reasons: vec![
                "partial scorer output; use evaluate_complete for acceptance".to_owned(),
            ],
        })
    }

    /// Compatibility alias for [`Self::score_partial`]. Use the named partial scorer for new
    /// diagnostics and [`Self::evaluate_complete`] for acceptance.
    pub fn evaluate(
        &self,
        decisions: &[SyntheticDecision],
    ) -> Result<SyntheticEvaluationReport, MapEvaluationError> {
        self.score_partial(decisions)
    }

    /// Scores and validates one complete row for every task and context mode.
    ///
    /// An image-unavailable row is valid evidence that the renderer was unavailable, but it
    /// leaves the returned report explicitly incomplete so callers cannot report image coverage.
    pub fn evaluate_complete(
        &self,
        decisions: &[SyntheticDecision],
    ) -> Result<SyntheticEvaluationReport, MapEvaluationError> {
        self.validate_complete_matrix(decisions)?;
        let mut report = self.score_partial(decisions)?;
        let mut incomplete_reasons = Vec::new();
        for row in &report.rows {
            if row.mode != ContextMode::GraphAnalysisImage {
                if row.image_bytes != 0 || row.image_status != "unavailable_not_requested" {
                    return Err(MapEvaluationError::InvalidDecision);
                }
                continue;
            }
            if row.image_status != "available" || row.image_bytes == 0 {
                incomplete_reasons.push(format!(
                    "task {} graph_analysis_image is {}",
                    row.task_id, row.image_status
                ));
            }
        }
        report.complete = incomplete_reasons.is_empty();
        report.incomplete_reasons = incomplete_reasons;
        Ok(report)
    }

    fn validate_complete_matrix(
        &self,
        decisions: &[SyntheticDecision],
    ) -> Result<(), MapEvaluationError> {
        let expected = self
            .bundles
            .len()
            .checked_mul(4)
            .ok_or(MapEvaluationError::TooManyDecisions)?;
        if decisions.len() != expected {
            return Err(MapEvaluationError::IncompleteMatrix(format!(
                "expected {expected} unique task-mode rows, received {}",
                decisions.len()
            )));
        }
        let mut seen = BTreeMap::<(&str, ContextMode), ()>::new();
        for decision in decisions {
            if !self.bundles.contains_key(&decision.task_id) {
                return Err(MapEvaluationError::UnknownTask(decision.task_id.clone()));
            }
            if seen
                .insert((decision.task_id.as_str(), decision.mode), ())
                .is_some()
            {
                return Err(MapEvaluationError::IncompleteMatrix(format!(
                    "duplicate task-mode row for {} {:?}",
                    decision.task_id, decision.mode
                )));
            }
        }
        for task_id in self.bundles.keys() {
            for mode in all_context_modes() {
                if !seen.contains_key(&(task_id.as_str(), mode)) {
                    return Err(MapEvaluationError::IncompleteMatrix(format!(
                        "missing task-mode row for {task_id} {mode:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn evaluate_one(
        &self,
        decision: &SyntheticDecision,
    ) -> Result<SyntheticEvaluationRow, MapEvaluationError> {
        validate_decision(decision)?;
        let bundle = self.bundle(&decision.task_id)?;
        let graph = SnapshotGraph::from_bundle(bundle)?;
        let best = graph.best_binding();
        let selected = graph.binding_for_action(&decision.proposed_action_id);
        let route_errors = graph.route_errors(&decision.proposed_route, selected);
        let binding_errors = selected.map_or(0, |binding| graph.binding_errors(binding));
        let topology_errors = route_errors.saturating_add(binding_errors);
        let invalid_proposals = u32::from(selected.is_none() || topology_errors > 0);
        let missed_route_opportunities = match (&best, selected) {
            (BestBinding::Exact(expected), Some(actual)) => {
                u32::from(expected.host_action_id != actual.host_action_id)
            }
            (BestBinding::Exact(_), None) => 1,
            (BestBinding::Unknown | BestBinding::Unavailable, _) => 0,
        };
        let (independent_best_action_id, independent_best_status) = match best {
            BestBinding::Exact(binding) => (Some(binding.host_action_id), "exact"),
            BestBinding::Unknown => (None, "unknown_overflow"),
            BestBinding::Unavailable => (None, "unavailable"),
        };
        let node_count =
            u32::try_from(graph.node_count()).map_err(|_| MapEvaluationError::TooLarge("nodes"))?;
        let edge_count =
            u32::try_from(graph.edge_count()).map_err(|_| MapEvaluationError::TooLarge("edges"))?;
        Ok(SyntheticEvaluationRow {
            task_id: decision.task_id.clone(),
            mode: decision.mode,
            topology_errors,
            missed_route_opportunities,
            invalid_proposals,
            request_bytes: decision.request_bytes,
            graph_bytes: decision.graph_bytes,
            analysis_bytes: decision.analysis_bytes,
            image_bytes: decision.image_bytes,
            image_status: decision.image_status.clone(),
            latency_micros: decision.latency_micros,
            node_count,
            edge_count,
            analysis_digest: bundle.manifest.analysis_digest.clone(),
            snapshot_digest: bundle.manifest.snapshot_digest.clone(),
            decision_action_id: decision.proposed_action_id.clone(),
            independent_best_action_id,
            independent_best_status: independent_best_status.to_owned(),
        })
    }
}

pub(crate) fn bundle_for_task(
    task: &SyntheticGraphTask,
) -> Result<MapViewBundle, MapEvaluationError> {
    let action_catalog_digest = crate::hex_bytes(Sha256::digest(task.task_id.as_bytes()));
    MapViewBundle::from_runtime_snapshot(
        task.snapshot.clone(),
        RuntimeMapBundleIdentity {
            run_id: format!("synthetic-run-{}", task.task_id),
            episode_id: format!("synthetic-episode-{}", task.task_id),
            trajectory_id: format!("synthetic-trajectory-{}", task.task_id),
            model_execution_id: None,
            action_catalog_digest,
        },
    )
    .map_err(MapEvaluationError::from)
}

fn validate_decision(decision: &SyntheticDecision) -> Result<(), MapEvaluationError> {
    if !valid_identifier(&decision.task_id)
        || decision.proposed_route.len() > SYNTHETIC_MAX_ROUTE_NODES
        || decision
            .proposed_route
            .iter()
            .any(|node| !valid_identifier(node))
        || !valid_action_identifier(&decision.proposed_action_id)
        || decision.request_bytes > SYNTHETIC_MAX_REQUEST_BYTES
        || decision.graph_bytes > SYNTHETIC_MAX_REQUEST_BYTES
        || decision.analysis_bytes > SYNTHETIC_MAX_REQUEST_BYTES
        || decision.image_bytes > SYNTHETIC_MAX_REQUEST_BYTES
        || decision.image_status.is_empty()
        || decision.image_status.len() > SYNTHETIC_MAX_IDENTIFIER_BYTES
        || decision.latency_micros > SYNTHETIC_MAX_LATENCY_MICROS
    {
        return Err(MapEvaluationError::InvalidDecision);
    }
    Ok(())
}

fn valid_action_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SYNTHETIC_MAX_ACTION_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SYNTHETIC_MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapEvaluationError {
    InvalidTasks,
    InvalidDecision,
    IncompleteMatrix(String),
    TooManyDecisions,
    TooLarge(&'static str),
    Serialization,
    UnknownTask(String),
    Bundle(String),
}

impl From<MapBundleError> for MapEvaluationError {
    fn from(error: MapBundleError) -> Self {
        Self::Bundle(error.to_string())
    }
}

impl fmt::Display for MapEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTasks => formatter.write_str("invalid synthetic map task"),
            Self::InvalidDecision => formatter.write_str("invalid synthetic map decision"),
            Self::IncompleteMatrix(reason) => {
                write!(
                    formatter,
                    "incomplete synthetic map evaluation matrix: {reason}"
                )
            }
            Self::TooManyDecisions => formatter.write_str("too many synthetic map decisions"),
            Self::TooLarge(field) => write!(formatter, "synthetic map {field} exceeds its bound"),
            Self::Serialization => formatter.write_str("synthetic map serialization failed"),
            Self::UnknownTask(task) => write!(formatter, "unknown synthetic map task {task}"),
            Self::Bundle(error) => write!(formatter, "synthetic map bundle rejected: {error}"),
        }
    }
}

impl std::error::Error for MapEvaluationError {}

#[must_use]
pub fn synthetic_action_id(destination: &str) -> String {
    format!("select-map-node:{destination}")
}
