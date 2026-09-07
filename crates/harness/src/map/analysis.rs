// SPDX-License-Identifier: MIT

use super::analysis_graph::{
    adjacency, category_distances, collect_reachable, distances, terminal_distances,
    topological_order,
};
use super::analysis_routes::candidate_routes;
use super::canonical::{CanonicalError, canonical_bytes, canonical_digest, reject_duplicate_keys};
use super::graph::{MapCompleteness, MapGraphError, ValidatedMapGraph};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAP_ANALYSIS_VERSION: &str = "sts2.map-analysis-v1";
pub const MAP_ANALYSIS_MAX_ASSUMPTIONS: usize = 32;
pub const MAP_ANALYSIS_MAX_CANDIDATES: usize = 8;
pub const MAP_ANALYSIS_MAX_ROUTE_NODES: usize = 256;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisConfig {
    pub evaluator_version: String,
    pub assumptions: Vec<String>,
    pub max_candidates: usize,
    pub max_count_digits: usize,
    pub max_route_nodes: usize,
}

/// Optional deterministic route preferences. Empty weights preserve the
/// structural ordering used by [`MapAnalysis::analyze`]. A positive weight
/// favors a category appearing on a route; a negative weight de-prioritizes it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePolicy {
    pub category_weights: BTreeMap<String, i32>,
    /// Additional weight applied once for each rest or campfire room.
    pub rest_weight: i32,
    /// Additional weight applied once for each shop or merchant room.
    pub shop_weight: i32,
    /// Additional weight applied once for each elite room.
    pub elite_weight: i32,
    /// Additional weight applied for each elite encountered before the first
    /// rest or campfire room.
    pub elite_exposure_before_rest_weight: i32,
}

impl RoutePolicy {
    fn validate(&self) -> Result<(), MapAnalysisError> {
        if self.category_weights.len() > 64
            || self.category_weights.iter().any(|(category, _)| {
                category.is_empty() || category.len() > super::graph::MAP_MAX_CATEGORY_BYTES
            })
        {
            return Err(MapAnalysisError::InvalidPolicy);
        }
        Ok(())
    }
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            evaluator_version: "structural-v1".to_owned(),
            assumptions: vec!["route scores use public topology only".to_owned()],
            max_candidates: MAP_ANALYSIS_MAX_CANDIDATES,
            max_count_digits: 4096,
            max_route_nodes: MAP_ANALYSIS_MAX_ROUTE_NODES,
        }
    }
}

impl AnalysisConfig {
    fn validate(&self) -> Result<(), MapAnalysisError> {
        if self.evaluator_version.is_empty()
            || self.evaluator_version.len() > 128
            || self.assumptions.len() > MAP_ANALYSIS_MAX_ASSUMPTIONS
            || self
                .assumptions
                .iter()
                .any(|value| value.is_empty() || value.len() > 512)
            || self.max_candidates == 0
            || self.max_candidates > MAP_ANALYSIS_MAX_CANDIDATES
            || self.max_count_digits == 0
            || self.max_count_digits > 16_384
            || self.max_route_nodes == 0
            || self.max_route_nodes > MAP_ANALYSIS_MAX_ROUTE_NODES
        {
            return Err(MapAnalysisError::InvalidConfig);
        }
        Ok(())
    }
}

impl MapAnalysis {
    pub fn analyze(
        graph: &ValidatedMapGraph,
        config: AnalysisConfig,
    ) -> Result<Self, MapAnalysisError> {
        Self::analyze_with_policy(graph, config, RoutePolicy::default())
    }

    pub fn analyze_with_policy(
        graph: &ValidatedMapGraph,
        config: AnalysisConfig,
        policy: RoutePolicy,
    ) -> Result<Self, MapAnalysisError> {
        config.validate()?;
        policy.validate()?;
        if matches!(graph.completeness(), MapCompleteness::Unavailable) {
            return Err(MapAnalysisError::Unavailable);
        }
        let (forward, reverse) = adjacency(graph);
        let order = topological_order(graph, &forward)?;
        let reachable = graph.current_node().map(|node| {
            let mut values = collect_reachable(node, &forward);
            values.insert(node.to_owned());
            values
        });
        let distances_from_current = graph.current_node().map(|node| distances(node, &forward));
        let distances_to_terminal = terminal_distances(graph, &forward, &order);
        let node_metrics = graph
            .nodes()
            .iter()
            .map(|node| NodeMetrics {
                node_id: node.node_id.clone(),
                reachable: reachable
                    .as_ref()
                    .is_some_and(|set| set.contains(&node.node_id)),
                distance_from_current: distances_from_current
                    .as_ref()
                    .and_then(|values| values.get(&node.node_id).copied()),
                distance_to_terminal: distances_to_terminal.get(&node.node_id).copied(),
                category_distances: category_distances(&node.node_id, graph, &forward),
                ancestors: collect_reachable(&node.node_id, &reverse)
                    .into_iter()
                    .collect(),
                descendants: collect_reachable(&node.node_id, &forward)
                    .into_iter()
                    .collect(),
            })
            .collect::<Vec<_>>();
        let terminal_counts = super::analysis_routes::terminal_counts(
            graph,
            &forward,
            &order,
            config.max_count_digits,
        );
        let legal_destination_counts = super::analysis_routes::legal_destination_counts(
            graph,
            &forward,
            &order,
            config.max_count_digits,
        );
        let (candidate_routes, candidates_bounded) = candidate_routes(
            graph,
            &forward,
            &distances_to_terminal,
            config.max_candidates,
            config.max_route_nodes,
            &policy,
        );
        let mut warnings = Vec::new();
        if matches!(graph.completeness(), MapCompleteness::Incomplete) {
            warnings.push("input graph is explicitly incomplete".to_owned());
        }
        if graph.current_node().is_none() {
            warnings.push("pre-start graph has no current node".to_owned());
        }
        if candidate_routes.is_empty() {
            warnings.push("no complete route reaches a visible terminal".to_owned());
        }
        if candidates_bounded {
            warnings
                .push("candidate routes are bounded and do not enumerate every route".to_owned());
        }
        let unbound_destinations = graph
            .current_node()
            .map(|current| {
                graph
                    .legal_destinations()
                    .iter()
                    .filter(|destination| {
                        destination.node_id != current
                            && !forward.get(current).is_some_and(|next| {
                                next.binary_search(&destination.node_id).is_ok()
                            })
                    })
                    .count()
            })
            .unwrap_or(0);
        if unbound_destinations > 0 {
            warnings.push(format!(
                "{unbound_destinations} legal destination binding(s) lack an explicit topology edge and were omitted"
            ));
        }
        let approximation = if matches!(graph.completeness(), MapCompleteness::Incomplete) {
            ApproximationStatus::IncompleteInput
        } else if candidates_bounded {
            ApproximationStatus::BoundedCandidates
        } else {
            ApproximationStatus::Exact
        };
        let current_node = graph.current_node();
        let mut branch_nodes = graph
            .nodes()
            .iter()
            .filter(|node| {
                Some(node.node_id.as_str()) != current_node
                    && forward
                        .get(&node.node_id)
                        .is_some_and(|next| next.len() > 1)
            })
            .map(|node| node.node_id.clone())
            .collect::<Vec<_>>();
        if let Some(current) = current_node
            && forward.get(current).is_some_and(|next| next.len() > 1)
        {
            branch_nodes.push(current.to_owned());
        }
        let merge_nodes = graph
            .nodes()
            .iter()
            .filter(|node| {
                reverse
                    .get(&node.node_id)
                    .is_some_and(|previous| previous.len() > 1)
            })
            .map(|node| node.node_id.clone())
            .collect();
        let mut analysis = Self {
            snapshot_digest: graph.snapshot_digest().to_owned(),
            analysis_version: MAP_ANALYSIS_VERSION.to_owned(),
            evaluator_version: config.evaluator_version,
            map_instance: graph.map_instance().to_owned(),
            act: graph.act().to_owned(),
            source_state_id: graph.source_state_id().to_owned(),
            generation: graph.generation(),
            assumptions: {
                let mut assumptions = config.assumptions;
                if !policy.category_weights.is_empty()
                    || policy.rest_weight != 0
                    || policy.shop_weight != 0
                    || policy.elite_weight != 0
                    || policy.elite_exposure_before_rest_weight != 0
                {
                    assumptions
                        .push("route category weights are deterministic preferences".to_owned());
                }
                assumptions
            },
            completeness: graph.completeness().clone(),
            approximation,
            topology: TopologySummary {
                node_count: graph.nodes().len(),
                edge_count: graph.edges().len(),
                current_node: graph.current_node().map(str::to_owned),
                reachable_nodes: reachable.as_ref().map_or(0, BTreeSet::len),
                branch_nodes,
                merge_nodes,
                cyclic: false,
            },
            node_metrics,
            terminal_counts,
            legal_destination_counts,
            candidate_routes,
            warnings,
            content_digest: String::new(),
        };
        analysis.content_digest = analysis.compute_digest()?;
        Ok(analysis)
    }

    fn compute_digest(&self) -> Result<String, MapAnalysisError> {
        let mut value = self.clone();
        value.content_digest.clear();
        canonical_digest(&value).map_err(MapAnalysisError::Canonical)
    }

    pub fn verify_digest(&self) -> Result<(), MapAnalysisError> {
        if self.content_digest != self.compute_digest()? {
            return Err(MapAnalysisError::DigestMismatch);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, MapAnalysisError> {
        self.verify_digest()?;
        canonical_bytes(self).map_err(MapAnalysisError::Canonical)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MapAnalysisError> {
        reject_duplicate_keys(bytes).map_err(MapAnalysisError::Canonical)?;
        let analysis: Self =
            serde_json::from_slice(bytes).map_err(|_| MapAnalysisError::Serialization)?;
        analysis.verify_digest()?;
        Ok(analysis)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapAnalysisError {
    InvalidConfig,
    InvalidPolicy,
    Unavailable,
    Cycle(Vec<String>),
    Serialization,
    DigestMismatch,
    Canonical(CanonicalError),
    Graph(MapGraphError),
}

impl fmt::Display for MapAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("invalid map analysis configuration"),
            Self::InvalidPolicy => formatter.write_str("invalid map route policy"),
            Self::Unavailable => formatter.write_str("map snapshot is unavailable"),
            Self::Cycle(nodes) => write!(formatter, "map graph contains a cycle: {nodes:?}"),
            Self::Serialization => formatter.write_str("map analysis serialization failed"),
            Self::DigestMismatch => formatter.write_str("map analysis digest mismatch"),
            Self::Canonical(error) => error.fmt(formatter),
            Self::Graph(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MapAnalysisError {}

impl From<MapGraphError> for MapAnalysisError {
    fn from(value: MapGraphError) -> Self {
        Self::Graph(value)
    }
}
