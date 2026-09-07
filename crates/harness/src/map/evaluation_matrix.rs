// SPDX-License-Identifier: MIT

use super::evaluation::{
    ContextMode, MapEvaluationError, SYNTHETIC_MAX_LATENCY_MICROS, SYNTHETIC_MAX_REQUEST_BYTES,
    SyntheticDecision, SyntheticEvaluationReport, SyntheticEvaluationRunner, SyntheticGraphTask,
    bundle_for_task,
};
use super::evaluation_renderer::{ImageCapture, render_image};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
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

fn parse_object(bytes: &[u8]) -> Result<Value, MapEvaluationError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| MapEvaluationError::Serialization)?;
    value
        .is_object()
        .then_some(value)
        .ok_or(MapEvaluationError::InvalidTasks)
}

/// Applies one deterministic adapter to the context actually delivered to the model.
///
/// A context without a graph can establish only the current node and one known hop. When the
/// graph is present, candidate bindings are checked against its adjacency and each candidate is
/// followed to a terminal. This keeps the adapter independent of the matrix mode and avoids
/// manufacturing route nodes that were not delivered in the context.
fn proposed_decision(context_bytes: &[u8]) -> Result<(String, Vec<String>), MapEvaluationError> {
    let context = parse_object(context_bytes)?;
    let graph_context = context.get("graph").is_some();
    let graph = context.get("graph").unwrap_or(&context);
    let position = graph
        .get("position")
        .and_then(|value| value.get("node_id"))
        .and_then(Value::as_str)
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let bindings = graph
        .get("bindings")
        .and_then(Value::as_array)
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let mut candidates = bindings
        .iter()
        .map(|binding| {
            Ok(BindingCandidate {
                graph_node_id: binding
                    .get("graph_node_id")
                    .and_then(Value::as_str)
                    .ok_or(MapEvaluationError::InvalidTasks)?
                    .to_owned(),
                host_action_id: binding
                    .get("host_action_id")
                    .and_then(Value::as_str)
                    .ok_or(MapEvaluationError::InvalidTasks)?
                    .to_owned(),
            })
        })
        .collect::<Result<Vec<_>, MapEvaluationError>>()?;
    candidates.sort_by(|left, right| {
        left.graph_node_id
            .cmp(&right.graph_node_id)
            .then_with(|| left.host_action_id.cmp(&right.host_action_id))
    });

    let Some(edges_value) = graph.get("edges") else {
        if graph_context {
            return Err(MapEvaluationError::InvalidTasks);
        }
        let candidate = candidates.first().ok_or(MapEvaluationError::InvalidTasks)?;
        return Ok((
            candidate.host_action_id.clone(),
            vec![position.to_owned(), candidate.graph_node_id.clone()],
        ));
    };
    let edges = edges_value
        .as_array()
        .ok_or(MapEvaluationError::InvalidTasks)?;
    let mut adjacency = BTreeMap::<String, Vec<String>>::new();
    for edge in edges {
        let from = edge
            .get("from")
            .and_then(Value::as_str)
            .ok_or(MapEvaluationError::InvalidTasks)?;
        let to = edge
            .get("to")
            .and_then(Value::as_str)
            .ok_or(MapEvaluationError::InvalidTasks)?;
        adjacency
            .entry(from.to_owned())
            .or_default()
            .push(to.to_owned());
    }
    for children in adjacency.values_mut() {
        children.sort();
        children.dedup();
    }
    let terminals = graph
        .get("terminal_node_ids")
        .and_then(Value::as_array)
        .ok_or(MapEvaluationError::InvalidTasks)?
        .iter()
        .map(|terminal| {
            terminal
                .as_str()
                .map(str::to_owned)
                .ok_or(MapEvaluationError::InvalidTasks)
        })
        .collect::<Result<BTreeSet<_>, MapEvaluationError>>()?;

    let mut routes = candidates
        .iter()
        .filter(|candidate| {
            adjacency
                .get(position)
                .is_some_and(|children| children.binary_search(&candidate.graph_node_id).is_ok())
        })
        .filter_map(|candidate| {
            let mut visited = BTreeSet::new();
            terminal_route(
                &candidate.graph_node_id,
                &adjacency,
                &terminals,
                &mut visited,
            )
            .map(|suffix| {
                let mut route = Vec::with_capacity(suffix.len() + 1);
                route.push(position.to_owned());
                route.extend(suffix);
                (route, candidate.host_action_id.clone())
            })
        })
        .collect::<Vec<_>>();
    routes.sort_by(|left, right| {
        left.0
            .len()
            .cmp(&right.0.len())
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.1.cmp(&right.1))
    });
    if let Some((route, action)) = routes.into_iter().next() {
        return Ok((action, route));
    }

    // The graph may be incomplete or no candidate may reach a declared terminal. Preserve the
    // one hop that was actually delivered so the scorer reports the missing continuation.
    let candidate = candidates.first().ok_or(MapEvaluationError::InvalidTasks)?;
    Ok((
        candidate.host_action_id.clone(),
        vec![position.to_owned(), candidate.graph_node_id.clone()],
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BindingCandidate {
    graph_node_id: String,
    host_action_id: String,
}

fn terminal_route(
    node: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    terminals: &BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Option<Vec<String>> {
    if !visited.insert(node.to_owned()) {
        return None;
    }
    if terminals.contains(node) {
        visited.remove(node);
        return Some(vec![node.to_owned()]);
    }
    let mut best = adjacency
        .get(node)
        .into_iter()
        .flatten()
        .filter_map(|child| {
            terminal_route(child, adjacency, terminals, visited).map(|suffix| {
                let mut route = Vec::with_capacity(suffix.len() + 1);
                route.push(node.to_owned());
                route.extend(suffix);
                route
            })
        })
        .collect::<Vec<_>>();
    visited.remove(node);
    best.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
    best.into_iter().next()
}

fn elapsed_micros(started: Instant) -> u64 {
    let nanos = started.elapsed().as_nanos();
    u64::try_from(nanos.div_ceil(1_000))
        .unwrap_or(SYNTHETIC_MAX_LATENCY_MICROS)
        .clamp(1, SYNTHETIC_MAX_LATENCY_MICROS)
}

fn matrix_snapshot() -> Vec<u8> {
    let nodes = [
        ("start", 0_i32, 0_i32, "start", true),
        ("left", 1, 0, "rest", false),
        ("right", 1, 1, "shop", false),
        ("boss", 2, 0, "boss", false),
    ]
    .into_iter()
    .map(|(id, row, column, category, visited)| {
        json!({"id":id,"row":row,"column":column,"category":category,"visited":visited})
    })
    .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "state_id":"matrix-state-v1",
        "generation":42,
        "schema_version":"visible-map-v1",
        "projection_version":"runtime-map-v1",
        "game_build":"synthetic-build",
        "mod_version":"synthetic-map-mod",
        "map_instance_id":"matrix-map-v1",
        "act_id":1,
        "scope_id":"matrix-scope-v1",
        "availability":"available",
        "completeness":"complete",
        "freshness":"current",
        "reason":null,
        "nodes":nodes,
        "edges":[
            {"from":"start","to":"left"},
            {"from":"start","to":"right"},
            {"from":"left","to":"boss"},
            {"from":"right","to":"boss"}
        ],
        "position":{"kind":"current","node_id":"start"},
        "history":["start"],
        "terminal_node_ids":["boss"],
        "bindings":[
            {"graph_node_id":"left","host_action_id":"select-map-node:42:matrix:left","action":{"kind":"select_map_node","node_id":"matrix-option:42:left"}},
            {"graph_node_id":"right","host_action_id":"select-map-node:42:matrix:right","action":{"kind":"select_map_node","node_id":"matrix-option:42:right"}}
        ]
    }))
    .unwrap_or_default()
}

fn bounded_u32(value: usize, field: &'static str) -> Result<u32, MapEvaluationError> {
    u32::try_from(value).map_err(|_| MapEvaluationError::TooLarge(field))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{
        ContextMode, SyntheticGraphTask, context_measurement, matrix_snapshot, parse_object,
        proposed_decision,
    };
    use serde_json::Value;

    #[test]
    fn adapter_is_driven_by_topology_and_stable_under_permutation() {
        let task =
            SyntheticGraphTask::new("adapter-original", matrix_snapshot()).expect("original task");
        let original_context = context_measurement(&task, ContextMode::Graph).expect("context");
        let (original_action, original_route) =
            proposed_decision(&original_context.bytes).expect("original decision");
        assert_eq!(original_action, "select-map-node:42:matrix:left");
        assert_eq!(original_route, ["start", "left", "boss"]);

        let mut permuted = parse_object(&task.snapshot).expect("snapshot object");
        let object = permuted.as_object_mut().expect("snapshot map");
        object
            .get_mut("edges")
            .and_then(Value::as_array_mut)
            .expect("edges")
            .reverse();
        object
            .get_mut("bindings")
            .and_then(Value::as_array_mut)
            .expect("bindings")
            .reverse();
        let permuted_task = SyntheticGraphTask::new(
            "adapter-permuted",
            serde_json::to_vec(&permuted).expect("permuted snapshot"),
        )
        .expect("permuted task");
        let permuted_context =
            context_measurement(&permuted_task, ContextMode::Graph).expect("permuted context");
        assert_eq!(
            proposed_decision(&permuted_context.bytes).expect("permuted decision"),
            (original_action.clone(), original_route.clone())
        );

        let mut changed = parse_object(&task.snapshot).expect("snapshot object");
        changed["edges"] = serde_json::json!([
            {"from":"start","to":"right"},
            {"from":"right","to":"left"},
            {"from":"left","to":"boss"}
        ]);
        let changed_task = SyntheticGraphTask::new(
            "adapter-changed",
            serde_json::to_vec(&changed).expect("changed snapshot"),
        )
        .expect("changed task");
        let changed_context =
            context_measurement(&changed_task, ContextMode::Graph).expect("changed context");
        let (changed_action, changed_route) =
            proposed_decision(&changed_context.bytes).expect("changed decision");
        assert_eq!(changed_action, "select-map-node:42:matrix:right");
        assert_eq!(changed_route, ["start", "right", "left", "boss"]);
        assert_ne!(changed_action, original_action);
    }
}
