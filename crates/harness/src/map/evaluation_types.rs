// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

/// The public context variants used by the offline evaluation matrix.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    NextMoveOnly,
    Graph,
    GraphAnalysis,
    GraphAnalysisImage,
}

/// One admitted public snapshot. The evaluator derives the graph and analysis from these bytes;
/// it does not accept an expected route supplied by a fixture author.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntheticGraphTask {
    pub task_id: String,
    pub snapshot: Vec<u8>,
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
