// SPDX-License-Identifier: MIT

#[path = "map_independent_oracle/oracle.rs"]
mod oracle;

use std::collections::{BTreeMap, BTreeSet};

use oracle::{
    OracleFixture, crossed_branch_fixture, cycle_fixture, digest_for, incomplete_fixture,
    layered_fixture, prestart_fixture, unbound_legal_destination_fixture,
};
use sts2_harness::{
    AnalysisConfig, ApproximationStatus, CountStatus, MAP_ANALYSIS_MAX_CANDIDATES, MapAnalysis,
    MapAnalysisError, MapCompleteness, MapGraphError,
};

fn config(max_candidates: usize, max_count_digits: usize) -> AnalysisConfig {
    AnalysisConfig {
        evaluator_version: "independent-oracle-test".to_owned(),
        assumptions: vec!["test expectations use a separately implemented oracle".to_owned()],
        max_candidates,
        max_count_digits,
        max_route_nodes: 256,
    }
}

fn analyze(
    fixture: &OracleFixture,
    max_candidates: usize,
    max_count_digits: usize,
) -> Result<MapAnalysis, MapAnalysisError> {
    let graph = fixture
        .to_validated_graph()
        .map_err(MapAnalysisError::Graph)?;
    MapAnalysis::analyze(&graph, config(max_candidates, max_count_digits))
}

fn terminal_counts(analysis: &MapAnalysis) -> BTreeMap<String, (&str, &str)> {
    analysis
        .terminal_counts
        .iter()
        .map(|count| {
            (
                count.terminal_id.clone(),
                (
                    count.count_decimal.as_str(),
                    match count.status {
                        CountStatus::Exact => "exact",
                        CountStatus::Overflow => "overflow",
                        CountStatus::Incomplete => "incomplete",
                    },
                ),
            )
        })
        .collect()
}

fn assert_candidate_is_concrete(fixture: &OracleFixture, analysis: &MapAnalysis) {
    let mut keys = BTreeSet::new();
    for candidate in &analysis.candidate_routes {
        assert!(fixture.route_is_concrete(&candidate.nodes));
        assert!(keys.insert(candidate.tie_break_key.clone()));
        assert_eq!(candidate.tie_break_key, candidate.nodes.join("\u{1f}"));
        assert_eq!(candidate.nodes.len() as u32, candidate.score.route_length);
        assert!(!candidate.first_action_id.is_empty());
    }
}

#[test]
fn small_dag_matches_independent_reverse_dp_and_exhaustive_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = crossed_branch_fixture("branch-merge-run", ("shop", "rest"));
    let analysis = analyze(&fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    let expected = fixture.reverse_path_counts("start")?;
    let counts = terminal_counts(&analysis);

    assert_eq!(analysis.topology.node_count, fixture.nodes.len());
    assert_eq!(analysis.topology.edge_count, fixture.edges.len());
    assert_eq!(
        analysis.topology.branch_nodes,
        vec!["left", "right", "merge", "start"]
    );
    assert_eq!(
        analysis.topology.merge_nodes,
        vec!["merge", "terminal-a", "terminal-b"]
    );
    assert_eq!(analysis.topology.reachable_nodes, 6);
    assert_eq!(analysis.topology.current_node.as_deref(), Some("start"));
    assert_eq!(analysis.approximation, ApproximationStatus::Exact);
    assert!(
        analysis
            .candidate_routes
            .iter()
            .all(|candidate| candidate.selection == ApproximationStatus::Exact)
    );

    for terminal in &fixture.terminals {
        let expected_decimal = expected
            .get(terminal)
            .copied()
            .ok_or("oracle omitted terminal")?
            .to_string();
        assert_eq!(
            counts.get(terminal),
            Some(&(expected_decimal.as_str(), "exact"))
        );
    }
    let current = analysis
        .node_metrics
        .iter()
        .find(|metric| metric.node_id == "start")
        .ok_or("current node metric missing")?;
    let orphan = analysis
        .node_metrics
        .iter()
        .find(|metric| metric.node_id == "orphan")
        .ok_or("disconnected node metric missing")?;
    assert!(current.reachable);
    assert!(!orphan.reachable);

    let exhaustive = fixture.legal_paths()?;
    let expected_route_count = exhaustive.values().map(Vec::len).sum::<usize>();
    assert_eq!(analysis.candidate_routes.len(), expected_route_count);
    assert_candidate_is_concrete(&fixture, &analysis);
    let produced = analysis
        .candidate_routes
        .iter()
        .map(|route| (route.first_action_id.clone(), route.nodes.clone()))
        .collect::<BTreeSet<_>>();
    let expected_routes = exhaustive
        .into_iter()
        .flat_map(|(action, routes)| routes.into_iter().map(move |route| (action.clone(), route)))
        .collect::<BTreeSet<_>>();
    assert_eq!(produced, expected_routes);
    let legal_route_counts = analysis.candidate_routes.iter().fold(
        BTreeMap::<String, usize>::new(),
        |mut counts, route| {
            *counts.entry(route.first_action_id.clone()).or_default() += 1;
            counts
        },
    );
    for destination in &fixture.legal_destinations {
        let expected_count = fixture
            .reverse_path_counts(&destination.node_id)?
            .values()
            .sum::<u128>() as usize;
        assert_eq!(
            legal_route_counts.get(&destination.action_id).copied(),
            Some(expected_count)
        );
    }

    let mut previous = None;
    for candidate in &analysis.candidate_routes {
        let key = (
            candidate.score.distance_to_terminal,
            std::cmp::Reverse(candidate.score.retained_branching),
            candidate.tie_break_key.as_str(),
        );
        if let Some(previous_key) = previous {
            assert!(previous_key <= key);
        }
        previous = Some(key);
    }
    Ok(())
}

#[test]
fn prestart_and_incomplete_states_remain_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let prestart = analyze(&prestart_fixture(), MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    assert_eq!(prestart.topology.current_node, None);
    assert_eq!(prestart.topology.reachable_nodes, 0);
    assert!(prestart.candidate_routes.is_empty());
    assert_eq!(prestart.approximation, ApproximationStatus::Exact);
    assert!(
        prestart
            .warnings
            .iter()
            .any(|warning| warning.contains("pre-start"))
    );
    assert!(
        prestart
            .warnings
            .iter()
            .any(|warning| warning.contains("no complete route"))
    );

    let incomplete_fixture = incomplete_fixture();
    let incomplete = analyze(&incomplete_fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    assert_eq!(
        incomplete.approximation,
        ApproximationStatus::IncompleteInput
    );
    assert!(
        incomplete
            .warnings
            .iter()
            .any(|warning| warning.contains("explicitly incomplete"))
    );
    assert!(
        incomplete
            .terminal_counts
            .iter()
            .all(|count| count.status == CountStatus::Incomplete)
    );
    assert_candidate_is_concrete(&incomplete_fixture, &incomplete);
    Ok(())
}

#[test]
fn terminal_order_is_not_allowed_to_hide_valid_routes() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = crossed_branch_fixture("unsorted-terminals-run", ("shop", "rest"));
    fixture.terminals.reverse();
    let analysis = analyze(&fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    let expected = fixture.reverse_path_counts("start")?;
    assert_eq!(analysis.candidate_routes.len(), 6);
    assert_candidate_is_concrete(&fixture, &analysis);
    for count in &analysis.terminal_counts {
        assert_eq!(count.status, CountStatus::Exact);
        assert_eq!(
            count.count_decimal,
            expected[&count.terminal_id].to_string()
        );
    }
    Ok(())
}

#[test]
fn malformed_graphs_and_cycles_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let cycle = cycle_fixture();
    let cycle_error = analyze(&cycle, MAP_ANALYSIS_MAX_CANDIDATES, 64)
        .err()
        .ok_or("cycle was accepted by analysis")?;
    assert!(matches!(cycle_error, MapAnalysisError::Cycle(_)));

    let duplicate = OracleFixture::from_parts(
        "duplicate-node-run",
        &[("same", 0, 0, "event"), ("same", 1, 1, "shop")],
        &[],
        None,
        &[],
        &[],
        MapCompleteness::Complete,
    );
    assert!(matches!(
        duplicate.to_validated_graph(),
        Err(MapGraphError::DuplicateNode(id)) if id == "same"
    ));

    let unknown_endpoint = OracleFixture::from_parts(
        "unknown-endpoint-run",
        &[("start", 0, 0, "start")],
        &[("start", "missing")],
        None,
        &[],
        &[],
        MapCompleteness::Complete,
    );
    assert!(matches!(
        unknown_endpoint.to_validated_graph(),
        Err(MapGraphError::UnknownEndpoint { from, to }) if from == "start" && to == "missing"
    ));
    Ok(())
}

#[test]
fn identities_survive_duplicate_coordinates_and_label_changes()
-> Result<(), Box<dyn std::error::Error>> {
    let first_fixture = crossed_branch_fixture("seed-reset-one", ("shop", "rest"));
    let second_fixture = crossed_branch_fixture("seed-reset-two", ("event", "elite"));
    let first = analyze(&first_fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    let second = analyze(&second_fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;

    assert_ne!(first.map_instance, second.map_instance);
    assert_eq!(first.topology.current_node, second.topology.current_node);
    assert_eq!(first.terminal_counts, second.terminal_counts);
    assert_eq!(
        first
            .candidate_routes
            .iter()
            .map(|route| route.nodes.clone())
            .collect::<Vec<_>>(),
        second
            .candidate_routes
            .iter()
            .map(|route| route.nodes.clone())
            .collect::<Vec<_>>()
    );
    assert_ne!(
        first_fixture
            .nodes
            .iter()
            .find(|node| node.id == "left")
            .map(|node| node.category.clone()),
        second_fixture
            .nodes
            .iter()
            .find(|node| node.id == "left")
            .map(|node| node.category.clone())
    );
    let duplicate_coordinates = first_fixture
        .nodes
        .iter()
        .filter(|node| node.row == 1 && node.column == 0)
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        duplicate_coordinates,
        BTreeSet::from(["left".to_owned(), "right".to_owned()])
    );
    assert_eq!(digest_for("seed-reset-one"), first_fixture.snapshot_digest);
    assert_ne!(
        first_fixture.snapshot_digest,
        second_fixture.snapshot_digest
    );
    Ok(())
}

#[test]
fn layered_graph_keeps_exponential_topology_without_exhaustive_analysis()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = layered_fixture(20, "layered-stress-run");
    let analysis = analyze(&fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    let expected = fixture.reverse_path_counts("start")?;
    let goal_count = expected.get("goal").copied().ok_or("goal count missing")?;
    assert_eq!(goal_count, 1_048_576);
    assert_eq!(analysis.topology.node_count, 42);
    assert_eq!(analysis.topology.edge_count, 80);
    let goal = analysis
        .terminal_counts
        .iter()
        .find(|count| count.terminal_id == "goal")
        .ok_or("goal terminal missing")?;
    assert_eq!(goal.count_decimal, "1048576");
    assert_eq!(goal.status, CountStatus::Exact);
    assert_eq!(analysis.candidate_routes.len(), MAP_ANALYSIS_MAX_CANDIDATES);
    assert_eq!(
        analysis.approximation,
        ApproximationStatus::BoundedCandidates
    );
    assert!(
        analysis
            .candidate_routes
            .iter()
            .all(|candidate| candidate.selection == ApproximationStatus::BoundedCandidates)
    );
    assert_candidate_is_concrete(&fixture, &analysis);
    assert!(
        analysis
            .warnings
            .iter()
            .any(|warning| warning.contains("bounded"))
    );
    Ok(())
}

#[test]
fn an_exhausted_candidate_budget_is_exact_when_the_oracle_finds_no_more_routes()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = layered_fixture(3, "exact-budget-run");
    let analysis = analyze(&fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    assert_eq!(fixture.reverse_path_counts("start")?["goal"], 8);
    assert_eq!(analysis.candidate_routes.len(), MAP_ANALYSIS_MAX_CANDIDATES);
    assert_eq!(analysis.approximation, ApproximationStatus::Exact);
    assert!(
        analysis
            .candidate_routes
            .iter()
            .all(|candidate| candidate.selection == ApproximationStatus::Exact)
    );
    assert_candidate_is_concrete(&fixture, &analysis);
    Ok(())
}

#[test]
fn overflow_and_bounded_selection_are_truthful_and_deterministic()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = layered_fixture(20, "overflow-run");
    let overflow = analyze(&fixture, MAP_ANALYSIS_MAX_CANDIDATES, 4)?;
    let goal = overflow
        .terminal_counts
        .iter()
        .find(|count| count.terminal_id == "goal")
        .ok_or("goal terminal missing")?;
    assert_eq!(goal.status, CountStatus::Overflow);
    assert_eq!(goal.count_decimal, "overflow");

    let small = crossed_branch_fixture("tie-run", ("shop", "rest"));
    let bounded = analyze(&small, 2, 64)?;
    assert_eq!(
        bounded.approximation,
        ApproximationStatus::BoundedCandidates
    );
    assert_eq!(bounded.candidate_routes.len(), 2);
    assert_candidate_is_concrete(&small, &bounded);
    assert!(
        bounded
            .warnings
            .iter()
            .any(|warning| warning.contains("bounded"))
    );

    let first = analyze(&small, 2, 64)?;
    let second = analyze(&small, 2, 64)?;
    assert_eq!(first.candidate_routes, second.candidate_routes);
    let mut reordered = small.clone();
    reordered.nodes.reverse();
    reordered.edges.reverse();
    let reordered_analysis = analyze(&reordered, 2, 64)?;
    assert_eq!(first.candidate_routes, reordered_analysis.candidate_routes);
    Ok(())
}

#[test]
fn legal_destination_without_an_edge_never_becomes_a_fictitious_route()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = unbound_legal_destination_fixture();
    let analysis = analyze(&fixture, MAP_ANALYSIS_MAX_CANDIDATES, 64)?;
    assert_candidate_is_concrete(&fixture, &analysis);
    assert!(analysis.candidate_routes.is_empty());
    Ok(())
}
