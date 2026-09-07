// SPDX-License-Identifier: MIT

use super::evaluation::{
    ContextMode, MapEvaluationError, SYNTHETIC_MAX_REQUEST_BYTES, SyntheticDecision,
    SyntheticEvaluationReport, SyntheticEvaluationRunner, SyntheticGraphTask, validate_task,
};
use super::graph::MapEdge;
use serde_json::json;

/// Serializes the public inputs visible in one synthetic context mode.
///
/// The resulting bytes are measurement inputs for `request_bytes`; they do not contain provider
/// output, token accounting, model claims, or a game action request.
pub fn synthetic_context_bytes(
    task: &SyntheticGraphTask,
    mode: ContextMode,
) -> Result<Vec<u8>, MapEvaluationError> {
    validate_task(task)?;
    let value = match mode {
        ContextMode::NextMoveOnly => json!({
            "context_mode": mode,
            "legal_destinations": task.legal_destinations,
        }),
        ContextMode::Graph => json!({
            "context_mode": mode,
            "nodes": task.nodes,
            "edges": task.edges,
        }),
        ContextMode::GraphAnalysis => json!({
            "context_mode": mode,
            "nodes": task.nodes,
            "edges": task.edges,
            "expected_routes": task.expected_routes,
        }),
        ContextMode::GraphAnalysisImage => json!({
            "context_mode": mode,
            "nodes": task.nodes,
            "edges": task.edges,
            "expected_routes": task.expected_routes,
            "image": {"height": 96, "width": 160, "format": "synthetic-map-v1"},
        }),
    };
    let bytes = serde_json::to_vec(&value).map_err(|_| MapEvaluationError::Serialization)?;
    if bytes.len() > SYNTHETIC_MAX_REQUEST_BYTES as usize {
        return Err(MapEvaluationError::TooLarge("serialized context"));
    }
    Ok(bytes)
}

/// Runs the bounded four-mode matrix against controlled public graph inputs.
///
/// Every row's bytes and latency are supplied by this deterministic fixture. The final image mode
/// deliberately proposes an unknown node, so the report exercises topology, missed-route, and
/// invalid-proposal counters rather than asserting a fabricated provider outcome.
pub fn run_bounded_synthetic_context_matrix()
-> Result<SyntheticEvaluationReport, MapEvaluationError> {
    let task = SyntheticGraphTask {
        task_id: "matrix-task-v1".to_owned(),
        nodes: vec![
            "start".to_owned(),
            "left".to_owned(),
            "right".to_owned(),
            "boss".to_owned(),
        ],
        edges: vec![
            MapEdge {
                from: "start".to_owned(),
                to: "left".to_owned(),
            },
            MapEdge {
                from: "start".to_owned(),
                to: "right".to_owned(),
            },
            MapEdge {
                from: "left".to_owned(),
                to: "boss".to_owned(),
            },
            MapEdge {
                from: "right".to_owned(),
                to: "boss".to_owned(),
            },
        ],
        legal_destinations: vec!["left".to_owned(), "right".to_owned()],
        expected_routes: vec![
            vec!["start".to_owned(), "left".to_owned(), "boss".to_owned()],
            vec!["start".to_owned(), "right".to_owned(), "boss".to_owned()],
        ],
    };
    let runner = SyntheticEvaluationRunner::new(vec![task.clone()])?;
    let modes = [
        ContextMode::NextMoveOnly,
        ContextMode::Graph,
        ContextMode::GraphAnalysis,
        ContextMode::GraphAnalysisImage,
    ];
    let latencies = [3_u32, 7, 11, 17];
    let decisions = modes
        .into_iter()
        .zip(latencies)
        .map(|(mode, latency_units)| {
            let context = synthetic_context_bytes(&task, mode)?;
            let proposed_route = if mode == ContextMode::GraphAnalysisImage {
                vec!["start".to_owned(), "unknown".to_owned(), "boss".to_owned()]
            } else if mode == ContextMode::Graph {
                vec!["start".to_owned(), "right".to_owned(), "boss".to_owned()]
            } else {
                vec!["start".to_owned(), "left".to_owned(), "boss".to_owned()]
            };
            let destination = proposed_route.get(1).cloned().unwrap_or_default();
            let request_bytes = u32::try_from(context.len())
                .map_err(|_| MapEvaluationError::TooLarge("serialized context"))?;
            Ok(SyntheticDecision {
                task_id: task.task_id.clone(),
                mode,
                proposed_route,
                proposed_action_id: super::evaluation::synthetic_action_id(&destination),
                request_bytes,
                latency_units,
            })
        })
        .collect::<Result<Vec<_>, MapEvaluationError>>()?;
    runner.evaluate(&decisions)
}
