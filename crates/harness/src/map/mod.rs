// SPDX-License-Identifier: MIT

mod analysis;
mod analysis_graph;
mod analysis_routes;
mod bundle;
mod bundle_store;
mod bundle_validation;
mod cache;
mod canonical;
mod evaluation;
mod evaluation_matrix;
mod feed;
mod graph;
mod wire;

pub use analysis::{
    AnalysisConfig, ApproximationStatus, CandidateRoute, CountStatus, LegalDestinationPathCount,
    MAP_ANALYSIS_MAX_CANDIDATES, MAP_ANALYSIS_VERSION, MapAnalysis, MapAnalysisError, NodeMetrics,
    RoutePolicy, RouteScore, TerminalPathCount, TopologySummary,
};
pub use bundle::{
    BundleContents, BundleHistory, BundleManifest, BundleOrigin, BundlePresentation,
    MAP_BUNDLE_VERSION, MAP_MAX_BUNDLE_BYTES, MAP_MAX_PNG_BYTES, MAP_MAX_PRESENTATION_HEIGHT,
    MAP_MAX_PRESENTATION_PIXELS, MAP_MAX_PRESENTATION_WIDTH, MAP_MAX_SNAPSHOT_BYTES,
    MAP_MIN_PRESENTATION_HEIGHT, MAP_MIN_PRESENTATION_WIDTH, MapBundleError, MapViewBundle,
    RUNTIME_MAP_SCHEMA_DIGEST, RUNTIME_MAP_UNRENDERED_DECISION, RuntimeMapBundleIdentity,
    build_unrendered_runtime_map_bundle,
};
pub use bundle_store::{
    BundleFileStore, HistoricalActionBinding, HistoricalReplay, MAP_MAX_FEED_ENTRIES,
    MapBundleFeed, PublicationReceipt,
};
pub use cache::{
    AnalysisCacheKey, BoundedCache, CacheError, InFlightGuard, MAP_CACHE_MAX_BYTES,
    MAP_CACHE_MAX_ENTRIES, MAP_CACHE_MAX_IN_FLIGHT, NavigationCacheKey, RenderCacheKey,
    TopologyCacheKey,
};
pub use evaluation::{
    ContextMode, MapEvaluationError, SYNTHETIC_MAX_DECISIONS, SYNTHETIC_MAX_EDGES,
    SYNTHETIC_MAX_IDENTIFIER_BYTES, SYNTHETIC_MAX_LATENCY_UNITS, SYNTHETIC_MAX_NODES,
    SYNTHETIC_MAX_REQUEST_BYTES, SYNTHETIC_MAX_ROUTE_NODES, SYNTHETIC_MAX_ROUTES,
    SYNTHETIC_MAX_TASKS, SyntheticDecision, SyntheticEvaluationReport, SyntheticEvaluationRow,
    SyntheticEvaluationRunner, SyntheticGraphTask, synthetic_action_id,
};
pub use evaluation_matrix::{run_bounded_synthetic_context_matrix, synthetic_context_bytes};
pub use feed::{MAP_FEED_FILE, MAP_FEED_VERSION, MapFeed, MapFeedEntry};
pub use graph::{
    LegalDestination, MAP_MAX_CATEGORY_BYTES, MAP_MAX_EDGES, MAP_MAX_IDENTIFIER_BYTES,
    MAP_MAX_NODES, MapCompleteness, MapEdge, MapGraphError, MapNode, MapNodeStatus,
    ValidatedMapGraph,
};
