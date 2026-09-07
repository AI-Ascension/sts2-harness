// SPDX-License-Identifier: MIT

use sts2_harness::{
    ContextMode, MapEdge, MapEvaluationError, SYNTHETIC_MAX_DECISIONS, SYNTHETIC_MAX_LATENCY_UNITS,
    SYNTHETIC_MAX_REQUEST_BYTES, SyntheticDecision, SyntheticEvaluationRunner, SyntheticGraphTask,
    run_bounded_synthetic_context_matrix, synthetic_action_id,
};

fn task() -> SyntheticGraphTask {
    SyntheticGraphTask {
        task_id: "task".to_owned(),
        nodes: vec![
            "start".to_owned(),
            "middle".to_owned(),
            "terminal".to_owned(),
        ],
        edges: vec![
            MapEdge {
                from: "start".to_owned(),
                to: "middle".to_owned(),
            },
            MapEdge {
                from: "middle".to_owned(),
                to: "terminal".to_owned(),
            },
        ],
        legal_destinations: vec!["middle".to_owned()],
        expected_routes: vec![vec![
            "start".to_owned(),
            "middle".to_owned(),
            "terminal".to_owned(),
        ]],
    }
}

fn decision(action_id: &str, route: Vec<&str>) -> SyntheticDecision {
    SyntheticDecision {
        task_id: "task".to_owned(),
        mode: ContextMode::Graph,
        proposed_route: route.into_iter().map(str::to_owned).collect(),
        proposed_action_id: action_id.to_owned(),
        request_bytes: 10,
        latency_units: 2,
    }
}

#[test]
fn bounded_matrix_has_four_modes_and_measures_only_controlled_inputs() {
    let report = run_bounded_synthetic_context_matrix().expect("bounded matrix");
    assert_eq!(report.evaluator_version, "synthetic-map-context-v2");
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
    assert!(report.rows.iter().all(|row| row.latency_units > 0));
    assert!(report.rows[..3].iter().all(|row| {
        row.topology_errors == 0
            && row.missed_route_opportunities == 0
            && row.invalid_proposals == 0
    }));
    let image = &report.rows[3];
    assert!(image.topology_errors > 0);
    assert_eq!(image.missed_route_opportunities, 1);
    assert_eq!(image.invalid_proposals, 1);
}

#[test]
fn fake_nonempty_action_is_invalid_even_when_route_is_valid() {
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let report = runner
        .evaluate(&[decision("fake-action", vec!["start", "middle", "terminal"])])
        .expect("evaluation");
    assert_eq!(report.rows[0].topology_errors, 0);
    assert_eq!(report.rows[0].invalid_proposals, 1);
}

#[test]
fn unknown_singleton_route_is_a_topology_and_proposal_error() {
    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let report = runner
        .evaluate(&[decision("fake-action", vec!["unknown"])])
        .expect("evaluation");
    assert!(report.rows[0].topology_errors > 0);
    assert_eq!(report.rows[0].invalid_proposals, 1);
    assert_eq!(report.rows[0].missed_route_opportunities, 1);
}

#[test]
fn malformed_graph_tasks_and_decision_bounds_are_rejected() {
    let mut duplicate_legal = task();
    duplicate_legal.legal_destinations.push("middle".to_owned());
    assert!(matches!(
        SyntheticEvaluationRunner::new(vec![duplicate_legal]),
        Err(MapEvaluationError::InvalidTasks)
    ));

    let mut malformed_expected = task();
    malformed_expected.expected_routes = vec![vec!["start".to_owned(), "terminal".to_owned()]];
    assert!(matches!(
        SyntheticEvaluationRunner::new(vec![malformed_expected]),
        Err(MapEvaluationError::InvalidTasks)
    ));

    let runner = SyntheticEvaluationRunner::new(vec![task()]).expect("task");
    let mut bounded = decision(
        &synthetic_action_id("middle"),
        vec!["start", "middle", "terminal"],
    );
    bounded.request_bytes = SYNTHETIC_MAX_REQUEST_BYTES + 1;
    assert_eq!(
        runner.evaluate(&[bounded]),
        Err(MapEvaluationError::InvalidDecision)
    );
    let mut bounded = decision(
        &synthetic_action_id("middle"),
        vec!["start", "middle", "terminal"],
    );
    bounded.latency_units = SYNTHETIC_MAX_LATENCY_UNITS + 1;
    assert_eq!(
        runner.evaluate(&[bounded]),
        Err(MapEvaluationError::InvalidDecision)
    );
    let decisions = vec![
        decision(
            &synthetic_action_id("middle"),
            vec!["start", "middle", "terminal"]
        );
        SYNTHETIC_MAX_DECISIONS + 1
    ];
    assert_eq!(
        runner.evaluate(&decisions),
        Err(MapEvaluationError::TooManyDecisions)
    );
}
