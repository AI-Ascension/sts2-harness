// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use sts2_harness::{
    AnalysisConfig, CountStatus, LegalDestination, MapAnalysis, MapCompleteness, MapEdge, MapNode,
    MapNodeStatus, RoutePolicy, ValidatedMapGraph,
};

fn diamond(categories: (&str, &str), legal: Vec<LegalDestination>) -> ValidatedMapGraph {
    ValidatedMapGraph::new(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "regression-map",
        "1",
        "regression-state",
        7,
        MapCompleteness::Complete,
        [
            ("start", 0, 0, "start", MapNodeStatus::Current),
            ("left", 1, 0, categories.0, MapNodeStatus::Available),
            ("right", 1, 1, categories.1, MapNodeStatus::Available),
            ("goal", 2, 0, "boss", MapNodeStatus::Unknown),
        ]
        .into_iter()
        .map(|(node_id, row, column, category, status)| MapNode {
            node_id: node_id.to_owned(),
            row,
            column,
            category: category.to_owned(),
            status,
        })
        .collect(),
        [
            ("start", "left"),
            ("start", "right"),
            ("left", "goal"),
            ("right", "goal"),
        ]
        .into_iter()
        .map(|(from, to)| MapEdge {
            from: from.to_owned(),
            to: to.to_owned(),
        })
        .collect(),
        Some("start".to_owned()),
        legal,
        vec!["goal".to_owned()],
    )
    .expect("valid regression graph")
}

fn config() -> AnalysisConfig {
    AnalysisConfig {
        evaluator_version: "regression".to_owned(),
        assumptions: vec!["fixture".to_owned()],
        max_candidates: 8,
        max_count_digits: 64,
        max_route_nodes: 256,
    }
}

fn destination(node_id: &str, action_id: &str) -> LegalDestination {
    LegalDestination {
        node_id: node_id.to_owned(),
        action_id: action_id.to_owned(),
    }
}

#[test]
fn current_component_and_legal_destination_counts_are_explicit() {
    let analysis = MapAnalysis::analyze(
        &diamond(
            ("rest", "elite"),
            vec![
                destination("left", "action-left"),
                destination("right", "action-right"),
            ],
        ),
        config(),
    )
    .expect("analysis");
    assert_eq!(analysis.topology.reachable_nodes, 4);
    assert!(
        analysis
            .node_metrics
            .iter()
            .find(|metric| metric.node_id == "start")
            .is_some_and(|metric| metric.reachable)
    );
    let counts = analysis
        .legal_destination_counts
        .iter()
        .map(|count| (count.action_id.as_str(), count.count_decimal.as_str()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(counts.get("action-left"), Some(&"1"));
    assert_eq!(counts.get("action-right"), Some(&"1"));
}

#[test]
fn unbound_legal_destination_is_incomplete_and_never_a_route() {
    let graph = ValidatedMapGraph::new(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "special-map",
        "1",
        "special-state",
        1,
        MapCompleteness::Complete,
        vec![
            MapNode {
                node_id: "start".to_owned(),
                row: 0,
                column: 0,
                category: "start".to_owned(),
                status: MapNodeStatus::Current,
            },
            MapNode {
                node_id: "special".to_owned(),
                row: 1,
                column: 1,
                category: "event".to_owned(),
                status: MapNodeStatus::Available,
            },
            MapNode {
                node_id: "goal".to_owned(),
                row: 2,
                column: 1,
                category: "boss".to_owned(),
                status: MapNodeStatus::Unknown,
            },
        ],
        vec![MapEdge {
            from: "special".to_owned(),
            to: "goal".to_owned(),
        }],
        Some("start".to_owned()),
        vec![destination("special", "action-special")],
        vec!["goal".to_owned()],
    )
    .expect("valid graph with a special binding");
    let analysis = MapAnalysis::analyze(&graph, config()).expect("analysis");
    assert!(analysis.candidate_routes.is_empty());
    assert_eq!(analysis.legal_destination_counts[0].count_decimal, "0");
    assert_eq!(
        analysis.legal_destination_counts[0].status,
        CountStatus::Incomplete
    );
    assert!(
        analysis
            .warnings
            .iter()
            .any(|warning| warning.contains("lack an explicit"))
    );
}

#[test]
fn category_policy_changes_only_the_deterministic_route_tie_break() {
    let mut policy = RoutePolicy::default();
    policy.category_weights.insert("rest".to_owned(), 10);
    policy.category_weights.insert("elite".to_owned(), -10);
    policy.rest_weight = 3;
    policy.elite_weight = -2;
    policy.elite_exposure_before_rest_weight = 4;
    let analysis = MapAnalysis::analyze_with_policy(
        &diamond(
            ("rest", "elite"),
            vec![
                destination("left", "action-left"),
                destination("right", "action-right"),
            ],
        ),
        config(),
        policy,
    )
    .expect("policy analysis");
    assert_eq!(analysis.candidate_routes[0].first_action_id, "action-left");
    assert!(
        analysis.candidate_routes[0].score.category_score
            > analysis.candidate_routes[1].score.category_score
    );
    assert_eq!(analysis.candidate_routes[0].score.rest_count, 1);
    assert_eq!(analysis.candidate_routes[0].score.shop_count, 0);
    assert_eq!(analysis.candidate_routes[0].score.elite_count, 0);
    assert_eq!(
        analysis.candidate_routes[0]
            .score
            .elite_exposure_before_rest,
        0
    );
    assert_eq!(
        analysis.candidate_routes[0].score.distance_to_terminal,
        Some(2)
    );
    assert_eq!(analysis.candidate_routes[1].score.rest_count, 0);
    assert_eq!(analysis.candidate_routes[1].score.elite_count, 1);
    assert_eq!(
        analysis.candidate_routes[1]
            .score
            .elite_exposure_before_rest,
        1
    );
}
