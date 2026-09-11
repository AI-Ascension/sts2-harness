// SPDX-License-Identifier: MIT

use super::graph::MapCompleteness;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApproximationStatus {
    Exact,
    BoundedCandidates,
    IncompleteInput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountStatus {
    Exact,
    Overflow,
    Incomplete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalPathCount {
    pub terminal_id: String,
    pub count_decimal: String,
    pub status: CountStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalDestinationPathCount {
    pub destination_node_id: String,
    pub action_id: String,
    pub count_decimal: String,
    pub status: CountStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologySummary {
    pub node_count: usize,
    pub edge_count: usize,
    pub current_node: Option<String>,
    pub reachable_nodes: usize,
    pub branch_nodes: Vec<String>,
    pub merge_nodes: Vec<String>,
    pub cyclic: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeMetrics {
    pub node_id: String,
    pub reachable: bool,
    pub distance_from_current: Option<u32>,
    pub distance_to_terminal: Option<u32>,
    pub category_distances: BTreeMap<String, u32>,
    pub ancestors: Vec<String>,
    pub descendants: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteScore {
    pub distance_to_terminal: Option<u32>,
    pub category_score: i32,
    /// Number of rest or campfire rooms on the concrete route.
    pub rest_count: u32,
    /// Number of shop or merchant rooms on the concrete route.
    pub shop_count: u32,
    /// Number of elite rooms on the concrete route.
    pub elite_count: u32,
    /// Number of elite rooms encountered before the first rest or campfire
    /// room on the concrete route. If the route has no rest room, this is the
    /// total elite count.
    pub elite_exposure_before_rest: u32,
    pub retained_branching: u32,
    pub route_length: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRoute {
    pub nodes: Vec<String>,
    pub first_action_id: String,
    pub score: RouteScore,
    pub tie_break_key: String,
    pub selection: ApproximationStatus,
    pub assumptions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapAnalysis {
    pub snapshot_digest: String,
    pub analysis_version: String,
    pub evaluator_version: String,
    pub map_instance: String,
    pub act: String,
    pub source_state_id: String,
    pub generation: u64,
    pub assumptions: Vec<String>,
    pub completeness: MapCompleteness,
    pub approximation: ApproximationStatus,
    pub topology: TopologySummary,
    pub node_metrics: Vec<NodeMetrics>,
    pub terminal_counts: Vec<TerminalPathCount>,
    pub legal_destination_counts: Vec<LegalDestinationPathCount>,
    pub candidate_routes: Vec<CandidateRoute>,
    pub warnings: Vec<String>,
    pub content_digest: String,
}
