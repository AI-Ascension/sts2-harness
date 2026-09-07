// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sts2_harness::{
    ContextMode, MapEvaluationError, SYNTHETIC_MAX_DECISIONS, SYNTHETIC_MAX_LATENCY_MICROS,
    SYNTHETIC_MAX_REQUEST_BYTES, SyntheticDecision, SyntheticEvaluationRunner, SyntheticGraphTask,
    run_bounded_synthetic_context_matrix, synthetic_context_bytes,
};

fn snapshot() -> Vec<u8> {
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
        "nodes":[
            {"id":"start","row":0,"column":0,"category":"start","visited":true},
            {"id":"left","row":1,"column":0,"category":"rest","visited":false},
            {"id":"right","row":1,"column":1,"category":"shop","visited":false},
            {"id":"boss","row":2,"column":0,"category":"boss","visited":false}
        ],
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
    .expect("synthetic snapshot serialization")
}

fn task() -> SyntheticGraphTask {
    SyntheticGraphTask::new("task", snapshot()).expect("synthetic task")
}

fn decision(action_id: &str, route: &[&str]) -> SyntheticDecision {
    SyntheticDecision {
        task_id: "task".to_owned(),
        mode: ContextMode::Graph,
        proposed_route: route.iter().map(|value| (*value).to_owned()).collect(),
        proposed_action_id: action_id.to_owned(),
        request_bytes: 10,
        graph_bytes: 4,
        analysis_bytes: 0,
        image_bytes: 0,
        image_status: "unavailable_not_requested".to_owned(),
        latency_micros: 2_000,
    }
}

#[test]
fn bounded_matrix_has_four_modes_and_measures_derived_inputs() {
    let report = run_bounded_synthetic_context_matrix().expect("bounded matrix");
    assert_eq!(report.evaluator_version, "synthetic-map-context-v3");
    assert_eq!(report.rows.len(), 4);
    assert_eq!(
        report.rows.iter().map(|row| row.mode).collect::<Vec<_>>(),
        vec![
            ContextMode::NextMoveOnly,
            ContextMode::Graph,
            ContextMode::GraphAnalysis,
            ContextMode::GraphAnalysisImage,
        ]
    );
    assert!(report.rows.iter().all(|row| row.request_bytes > 0));
    assert!(report.rows.iter().all(|row| row.latency_micros > 0));
    assert_eq!(report.rows[0].graph_bytes, 0);
    assert!(report.rows[1].graph_bytes > 0);
    assert!(report.rows[2].analysis_bytes > 0);
    assert!(report.rows[3].analysis_bytes > 0);
    assert_eq!(
        report.complete,
        report.rows[3].image_status == "available",
        "image availability must control matrix completeness"
    );
    if !report.complete {
        assert!(!report.incomplete_reasons.is_empty());
    }
    assert!(
        report.rows[3].image_status == "available"
            || report.rows[3].image_status.starts_with("unavailable_")
    );
    assert!(
        report.rows[..3]
            .iter()
            .all(|row| { row.topology_errors == 0 && row.invalid_proposals == 0 })
    );
    assert_eq!(report.rows[1].missed_route_opportunities, 0);
    assert_eq!(report.rows[2].missed_route_opportunities, 0);
}

#[test]
fn context_modes_include_only_the_declared_public_material() {
    let task = task();
    let next = synthetic_context_bytes(&task, ContextMode::NextMoveOnly).expect("next context");
    let graph = synthetic_context_bytes(&task, ContextMode::Graph).expect("graph context");
    let analysis =
        synthetic_context_bytes(&task, ContextMode::GraphAnalysis).expect("analysis context");
    assert!(next.len() < graph.len());
    assert!(graph.len() < analysis.len());
    let next_value: Value = serde_json::from_slice(&next).expect("next JSON");
    assert!(next_value.get("nodes").is_none());
    assert!(next_value.get("bindings").is_some());
    let graph_value: Value = serde_json::from_slice(&graph).expect("graph JSON");
    assert!(graph_value["graph"]["nodes"].is_array());
    assert!(graph_value["graph"]["edges"].is_array());
    assert!(graph_value.get("analysis").is_none());
    let analysis_value: Value = serde_json::from_slice(&analysis).expect("analysis JSON");
    assert!(analysis_value.get("analysis").is_some());
}

#[test]
fn fake_action_is_invalid_even_when_route_topology_is_valid() {
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let report = runner
        .evaluate(&[decision("fake-action", &["start", "left", "boss"])])
        .expect("evaluation");
    assert_eq!(report.rows[0].topology_errors, 0);
    assert_eq!(report.rows[0].invalid_proposals, 1);
}

#[test]
fn unknown_route_is_a_topology_and_proposal_error() {
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let report = runner
        .evaluate(&[decision("fake-action", &["start", "unknown", "boss"])])
        .expect("evaluation");
    assert!(report.rows[0].topology_errors > 0);
    assert_eq!(report.rows[0].invalid_proposals, 1);
    assert_eq!(report.rows[0].missed_route_opportunities, 1);
}

#[test]
fn host_action_ids_can_exceed_graph_identifier_bound() {
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let mut long_action = decision(
        &format!("{}select-map-node", "a".repeat(140)),
        &["start", "left", "boss"],
    );
    long_action.proposed_action_id = "a".repeat(144);
    let report = runner
        .evaluate(&[long_action])
        .expect("long action evaluation");
    assert_eq!(report.rows[0].invalid_proposals, 1);
}

#[test]
fn malformed_tasks_and_decision_bounds_are_rejected() {
    assert!(matches!(
        SyntheticGraphTask::new("bad task", snapshot()),
        Err(MapEvaluationError::InvalidTasks)
    ));
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let mut bounded = decision("select-map-node:42:matrix:left", &["start", "left", "boss"]);
    bounded.request_bytes = SYNTHETIC_MAX_REQUEST_BYTES + 1;
    assert_eq!(
        runner.evaluate(&[bounded]),
        Err(MapEvaluationError::InvalidDecision)
    );
    let mut bounded = decision("select-map-node:42:matrix:left", &["start", "left", "boss"]);
    bounded.latency_micros = SYNTHETIC_MAX_LATENCY_MICROS + 1;
    assert_eq!(
        runner.evaluate(&[bounded]),
        Err(MapEvaluationError::InvalidDecision)
    );
    let decisions = vec![
        decision("select-map-node:42:matrix:left", &["start", "left", "boss"],);
        SYNTHETIC_MAX_DECISIONS + 1
    ];
    assert_eq!(
        runner.evaluate(&decisions),
        Err(MapEvaluationError::TooManyDecisions)
    );
}

#[test]
fn complete_matrix_rejects_missing_and_duplicate_rows() {
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let rows = [
        ContextMode::NextMoveOnly,
        ContextMode::Graph,
        ContextMode::GraphAnalysis,
        ContextMode::GraphAnalysisImage,
    ]
    .map(|mode| SyntheticDecision {
        task_id: "task".to_owned(),
        mode,
        proposed_route: vec!["start".to_owned(), "left".to_owned(), "boss".to_owned()],
        proposed_action_id: "select-map-node:42:matrix:left".to_owned(),
        request_bytes: 10,
        graph_bytes: u32::from(mode != ContextMode::NextMoveOnly),
        analysis_bytes: u32::from(matches!(
            mode,
            ContextMode::GraphAnalysis | ContextMode::GraphAnalysisImage
        )),
        image_bytes: 0,
        image_status: if mode == ContextMode::GraphAnalysisImage {
            "unavailable_renderer_output".to_owned()
        } else {
            "unavailable_not_requested".to_owned()
        },
        latency_micros: 1,
    });
    assert!(matches!(
        runner.evaluate_complete(&rows[..3]),
        Err(MapEvaluationError::IncompleteMatrix(_))
    ));
    let mut duplicate = rows.clone();
    duplicate[3].mode = ContextMode::Graph;
    assert!(matches!(
        runner.evaluate_complete(&duplicate),
        Err(MapEvaluationError::IncompleteMatrix(_))
    ));
    let report = runner
        .evaluate_complete(&rows)
        .expect("unavailable image report");
    assert!(!report.complete);
    assert_eq!(report.rows.len(), 4);
    assert!(!report.incomplete_reasons.is_empty());
}
