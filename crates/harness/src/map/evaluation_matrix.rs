// SPDX-License-Identifier: MIT

use super::evaluation::{
    ContextMode, MapEvaluationError, SYNTHETIC_MAX_LATENCY_MICROS, SYNTHETIC_MAX_REQUEST_BYTES,
    SyntheticDecision, SyntheticEvaluationReport, SyntheticEvaluationRunner, SyntheticGraphTask,
    bundle_for_task,
};
use super::evaluation_fixture::matrix_snapshot;
use super::evaluation_renderer::{ImageCapture, render_image};
use super::evaluation_routes::{parse_object, proposed_decision};
use serde_json::{Map, Value, json};
use std::time::Instant;

/// Serializes the public values exposed by one context mode. The graph and analysis are derived
/// from the admitted snapshot; fixture authors cannot provide an expected route as an oracle.
pub fn synthetic_context_bytes(
    task: &SyntheticGraphTask,
    mode: ContextMode,
) -> Result<Vec<u8>, MapEvaluationError> {
    Ok(context_measurement(task, mode)?.bytes)
}

/// Runs the bounded four-mode matrix against one controlled public snapshot. Graph and analysis
/// bytes are measured from the actual snapshot and harness analysis, while each row records the
/// elapsed time spent assembling that mode and producing its deterministic proposal. Image mode
/// invokes the configured renderer when its binary and digest are present, and records an
/// explicit unavailable status otherwise.
pub fn run_bounded_synthetic_context_matrix()
-> Result<SyntheticEvaluationReport, MapEvaluationError> {
    let task = SyntheticGraphTask::new("matrix-task-v1", matrix_snapshot())?;
    let runner = SyntheticEvaluationRunner::new(vec![task.clone()])?;
    let modes = [
        ContextMode::NextMoveOnly,
        ContextMode::Graph,
        ContextMode::GraphAnalysis,
        ContextMode::GraphAnalysisImage,
    ];
    let decisions = modes
        .into_iter()
        .map(|mode| {
            let started = Instant::now();
            let measurement = context_measurement(&task, mode)?;
            let (proposed_action_id, proposed_route) = proposed_decision(&measurement.bytes)?;
            let latency_micros = elapsed_micros(started);
            let request_bytes = bounded_u32(measurement.bytes.len(), "serialized context")?;
            Ok(SyntheticDecision {
                task_id: task.task_id.clone(),
                mode,
                proposed_route,
                proposed_action_id,
                request_bytes,
                graph_bytes: bounded_u32(measurement.graph_bytes, "graph context")?,
                analysis_bytes: bounded_u32(measurement.analysis_bytes, "analysis context")?,
                image_bytes: bounded_u32(measurement.image_bytes, "image")?,
                image_status: measurement.image_status,
                latency_micros,
            })
        })
        .collect::<Result<Vec<_>, MapEvaluationError>>()?;
    runner.evaluate_complete(&decisions)
}

struct ContextMeasurement {
    bytes: Vec<u8>,
    graph_bytes: usize,
    analysis_bytes: usize,
    image_bytes: usize,
    image_status: String,
}

fn context_measurement(
    task: &SyntheticGraphTask,
    mode: ContextMode,
) -> Result<ContextMeasurement, MapEvaluationError> {
    let bundle = bundle_for_task(task)?;
    let snapshot = parse_object(&bundle.snapshot_bytes)?;
    let graph = graph_value(&snapshot)?;
    let graph_bytes = serde_json::to_vec(&graph).map_err(|_| MapEvaluationError::Serialization)?;
    let analysis = if matches!(
        mode,
        ContextMode::GraphAnalysis | ContextMode::GraphAnalysisImage
    ) {
        Some(
            bundle
                .analysis_bytes()
                .map_err(|_| MapEvaluationError::Serialization)?,
        )
    } else {
        None
    };
    let image = if mode == ContextMode::GraphAnalysisImage {
        render_image(&bundle)
    } else {
        ImageCapture::unavailable("unavailable_not_requested")
    };

    let mut context = Map::new();
    context.insert(
        "context_mode".to_owned(),
        serde_json::to_value(mode).map_err(|_| MapEvaluationError::Serialization)?,
    );
    context.insert(
        "snapshot_digest".to_owned(),
        Value::String(bundle.manifest.snapshot_digest.clone()),
    );
    match mode {
        ContextMode::NextMoveOnly => {
            context.insert(
                "position".to_owned(),
                snapshot
                    .get("position")
                    .cloned()
                    .ok_or(MapEvaluationError::InvalidTasks)?,
            );
            context.insert(
                "bindings".to_owned(),
                snapshot
                    .get("bindings")
                    .cloned()
                    .ok_or(MapEvaluationError::InvalidTasks)?,
            );
            context.insert(
                "freshness".to_owned(),
                snapshot
                    .get("freshness")
                    .cloned()
                    .ok_or(MapEvaluationError::InvalidTasks)?,
            );
        }
        ContextMode::Graph | ContextMode::GraphAnalysis | ContextMode::GraphAnalysisImage => {
            context.insert("graph".to_owned(), graph);
            if let Some(analysis_bytes) = &analysis {
                context.insert(
                    "analysis".to_owned(),
                    serde_json::from_slice(analysis_bytes)
                        .map_err(|_| MapEvaluationError::Serialization)?,
                );
            }
            if mode == ContextMode::GraphAnalysisImage {
                if let Some(image) = image.value {
                    context.insert("image".to_owned(), image);
                } else {
                    context.insert(
                        "image".to_owned(),
                        json!({"media_type":"image/png", "status": image.status}),
                    );
                }
            }
        }
    }
    let bytes = serde_json::to_vec(&Value::Object(context))
        .map_err(|_| MapEvaluationError::Serialization)?;
    if bytes.len() > SYNTHETIC_MAX_REQUEST_BYTES as usize {
        return Err(MapEvaluationError::TooLarge("serialized context"));
    }
    let graph_bytes_len = if matches!(
        mode,
        ContextMode::Graph | ContextMode::GraphAnalysis | ContextMode::GraphAnalysisImage
    ) {
        graph_bytes.len()
    } else {
        0
    };
    Ok(ContextMeasurement {
        bytes,
        graph_bytes: graph_bytes_len,
        analysis_bytes: analysis.as_ref().map_or(0, Vec::len),
        image_bytes: image.bytes,
        image_status: image.status,
    })
}

fn graph_value(snapshot: &Value) -> Result<Value, MapEvaluationError> {
    let object = snapshot
        .as_object()
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let mut graph = Map::new();
    for key in [
        "nodes",
        "edges",
        "position",
        "history",
        "terminal_node_ids",
        "bindings",
        "completeness",
        "freshness",
    ] {
        graph.insert(
            key.to_owned(),
            object
                .get(key)
                .cloned()
                .ok_or(MapEvaluationError::InvalidTasks)?,
        );
    }
    Ok(Value::Object(graph))
}

fn elapsed_micros(started: Instant) -> u64 {
    let nanos = started.elapsed().as_nanos();
    u64::try_from(nanos.div_ceil(1_000))
        .unwrap_or(SYNTHETIC_MAX_LATENCY_MICROS)
        .clamp(1, SYNTHETIC_MAX_LATENCY_MICROS)
}

fn bounded_u32(value: usize, field: &'static str) -> Result<u32, MapEvaluationError> {
    u32::try_from(value).map_err(|_| MapEvaluationError::TooLarge(field))
}

#[cfg(test)]
#[path = "evaluation_matrix_tests.rs"]
mod tests;
