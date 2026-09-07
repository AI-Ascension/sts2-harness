// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::PathBuf;
use sts2_harness::{MapViewBundle, RuntimeMapBundleIdentity};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/map-bundle-demo-v1")
}

pub fn demo_snapshot() -> Vec<u8> {
    let categories = [
        "start", "event", "monster", "elite", "rest", "monster", "event", "shop", "elite", "rest",
        "event", "shop", "monster", "event", "boss",
    ];
    let mut nodes = Vec::new();
    for row in 0..15 {
        for lane in 0_usize..5 {
            nodes.push(json!({
                "category": categories[row],
                "column": lane,
                "id": format!("demo:{row}:{lane}"),
                "row": row,
                "visited": row < 2,
            }));
        }
    }
    nodes.push(json!({
        "category": "event",
        "column": 7,
        "id": "demo:unreachable:0",
        "row": 15,
        "visited": false,
    }));
    let mut edges = Vec::new();
    for row in 0..14 {
        for lane in 0_usize..5 {
            for next_lane in lane.saturating_sub(1)..=(lane + 1).min(4) {
                edges.push(json!({
                    "from": format!("demo:{row}:{lane}"),
                    "to": format!("demo:{}:{next_lane}", row + 1),
                }));
            }
        }
    }
    let bindings = [1, 2, 3]
        .into_iter()
        .map(|lane| {
            json!({
                "action": {
                    "kind": "select_map_node",
                    "node_id": format!("demo-option:42:{lane}"),
                },
                "graph_node_id": format!("demo:3:{lane}"),
                "host_action_id": format!("select-map-node:42:demo:3:{lane}"),
            })
        })
        .collect::<Vec<_>>();
    let terminals = (0..5)
        .map(|lane| format!("demo:14:{lane}"))
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "act_id": 1,
        "availability": "available",
        "bindings": bindings,
        "completeness": "complete",
        "edges": edges,
        "freshness": "current",
        "game_build": "demo-build",
        "generation": 42,
        "history": ["demo:0:2", "demo:1:1", "demo:1:2"],
        "map_instance_id": "demo-map",
        "mod_version": "demo-map-mod",
        "nodes": nodes,
        "position": {"kind": "current", "node_id": "demo:2:2"},
        "projection_version": "runtime-map-v1",
        "reason": Value::Null,
        "schema_version": "visible-map-v1",
        "scope_id": "demo-scope",
        "state_id": "demo-state-42",
        "terminal_node_ids": terminals,
    }))
    .expect("demo snapshot JSON")
}

#[test]
fn dense_demo_fixture_is_generated_by_the_harness_builder() {
    let root = fixture_root();
    let feed: Value = serde_json::from_slice(&fs::read(root.join("feed.json")).expect("demo feed"))
        .expect("demo feed JSON");
    let digest = feed["head"].as_str().expect("demo feed head");
    let directory = root.join(digest);
    let snapshot = fs::read(directory.join("visible-map.json")).expect("demo snapshot");
    assert_eq!(snapshot, demo_snapshot());
    let bundle = MapViewBundle::from_runtime_snapshot(
        snapshot.clone(),
        RuntimeMapBundleIdentity {
            run_id: "demo-run-001".to_owned(),
            episode_id: "demo-episode-001".to_owned(),
            trajectory_id: "demo-trajectory-001".to_owned(),
            model_execution_id: Some("demo-model-execution-001".to_owned()),
            action_catalog_digest: format!("{:x}", Sha256::digest(b"demo-action-catalog-v1")),
        },
    )
    .expect("demo bundle");
    assert_eq!(bundle.manifest.bundle_digest, digest);
    assert_eq!(bundle.manifest.renderer_version, "unrendered");
    assert_eq!(bundle.viewer.as_deref(), Some(b"{}".as_slice()));
    assert_eq!(bundle.manifest.presentation, None);
    assert_eq!(bundle.analysis.topology.node_count, 76);
    assert_eq!(bundle.analysis.topology.edge_count, 182);
    assert!(bundle.analysis.topology.reachable_nodes < bundle.analysis.topology.node_count);
    assert!(bundle.analysis.candidate_routes.len() <= 8);
    assert!(bundle.analysis.candidate_routes.iter().any(|route| {
        route.score.rest_count > 0
            && route.score.shop_count > 0
            && route.score.elite_count > 0
            && route.score.elite_exposure_before_rest > 0
    }));
    assert_eq!(
        fs::read(directory.join("analysis.json")).expect("demo analysis"),
        bundle.analysis_bytes().expect("demo analysis bytes")
    );
    assert_eq!(
        fs::read(directory.join("manifest.json")).expect("demo manifest"),
        bundle
            .canonical_manifest_bytes()
            .expect("demo manifest bytes")
    );
}
