// SPDX-License-Identifier: MIT

use super::OracleFixture;
use sts2_harness::MapCompleteness;

pub fn crossed_branch_fixture(map_instance: &str, labels: (&str, &str)) -> OracleFixture {
    OracleFixture::from_parts(
        map_instance,
        &[
            ("start", 0, 0, "start"),
            ("left", 1, 0, labels.0),
            ("right", 1, 0, labels.1),
            ("merge", 2, 0, "event"),
            ("terminal-a", 3, -1, "terminal"),
            ("terminal-b", 3, 1, "terminal"),
            ("orphan", 1, 2, "unknown"),
        ],
        &[
            ("right", "merge"),
            ("start", "right"),
            ("left", "terminal-b"),
            ("merge", "terminal-b"),
            ("start", "left"),
            ("merge", "terminal-a"),
            ("right", "terminal-a"),
            ("left", "merge"),
        ],
        Some("start"),
        &[("left", "action-left"), ("right", "action-right")],
        &["terminal-a", "terminal-b"],
        MapCompleteness::Complete,
    )
}

pub fn layered_fixture(layers: usize, map_instance: &str) -> OracleFixture {
    let mut nodes = vec![("start".to_owned(), 0, 0, "start".to_owned())];
    for layer in 0..layers {
        nodes.push((
            format!("layer-{layer}-a"),
            i32::try_from(layer + 1).unwrap_or(i32::MAX),
            0,
            "unknown".to_owned(),
        ));
        nodes.push((
            format!("layer-{layer}-b"),
            i32::try_from(layer + 1).unwrap_or(i32::MAX),
            1,
            "unknown".to_owned(),
        ));
    }
    nodes.push((
        "goal".to_owned(),
        i32::try_from(layers + 1).unwrap_or(i32::MAX),
        0,
        "boss".to_owned(),
    ));
    let mut edges = Vec::new();
    if layers == 0 {
        edges.push(("start".to_owned(), "goal".to_owned()));
    } else {
        edges.push(("start".to_owned(), "layer-0-a".to_owned()));
        edges.push(("start".to_owned(), "layer-0-b".to_owned()));
        for layer in 0..layers.saturating_sub(1) {
            for from in ["a", "b"] {
                for to in ["a", "b"] {
                    edges.push((
                        format!("layer-{layer}-{from}"),
                        format!("layer-{}-{to}", layer + 1),
                    ));
                }
            }
        }
        edges.push((format!("layer-{}-a", layers - 1), "goal".to_owned()));
        edges.push((format!("layer-{}-b", layers - 1), "goal".to_owned()));
    }
    let node_refs = nodes
        .iter()
        .map(|(id, row, column, category)| (id.as_str(), *row, *column, category.as_str()))
        .collect::<Vec<_>>();
    let edge_refs = edges
        .iter()
        .map(|(from, to)| (from.as_str(), to.as_str()))
        .collect::<Vec<_>>();
    let first_a = if layers == 0 { "goal" } else { "layer-0-a" };
    let first_b = if layers == 0 { "goal" } else { "layer-0-b" };
    OracleFixture::from_parts(
        map_instance,
        &node_refs,
        &edge_refs,
        Some("start"),
        &[(first_a, "action-a"), (first_b, "action-b")],
        &["goal"],
        MapCompleteness::Complete,
    )
}

pub fn cycle_fixture() -> OracleFixture {
    OracleFixture::from_parts(
        "cycle-run",
        &[("a", 0, 0, "event"), ("b", 1, 0, "event")],
        &[("a", "b"), ("b", "a")],
        Some("a"),
        &[("b", "action-b")],
        &["b"],
        MapCompleteness::Complete,
    )
}

pub fn prestart_fixture() -> OracleFixture {
    OracleFixture::from_parts(
        "prestart-run",
        &[("start", 0, 0, "start"), ("terminal", 1, 0, "boss")],
        &[("start", "terminal")],
        None,
        &[],
        &["terminal"],
        MapCompleteness::Complete,
    )
}

pub fn incomplete_fixture() -> OracleFixture {
    let mut fixture = crossed_branch_fixture("incomplete-run", ("shop", "rest"));
    fixture.completeness = MapCompleteness::Incomplete;
    fixture
}

pub fn unbound_legal_destination_fixture() -> OracleFixture {
    OracleFixture::from_parts(
        "special-move-run",
        &[
            ("start", 0, 0, "start"),
            ("special", 1, 4, "unknown"),
            ("goal", 2, 4, "boss"),
        ],
        &[("special", "goal")],
        Some("start"),
        &[("special", "action-special")],
        &["goal"],
        MapCompleteness::Complete,
    )
}
