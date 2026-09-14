// SPDX-License-Identifier: MIT

use serde_json::json;

pub(super) fn map_request_bytes() -> Vec<u8> {
    let mut nodes = vec![json!({
        "id": "start",
        "row": 0,
        "column": 0,
        "category": "start",
        "visited": true
    })];
    let mut node_ids = vec![String::from("start")];
    for index in 1..256 {
        let id = format!("z{index:03}{}", "a".repeat(124));
        node_ids.push(id.clone());
        nodes.push(json!({
            "id": id,
            "row": index,
            "column": 0,
            "category": "monster",
            "visited": false
        }));
    }
    let mut edges = Vec::new();
    'outer: for from in 0..node_ids.len() {
        for to in (from + 1)..node_ids.len() {
            edges.push(json!({"from": node_ids[from], "to": node_ids[to]}));
            if edges.len() == 600 {
                break 'outer;
            }
        }
    }
    let snapshot = json!({
        "state_id": "state-1", "generation": 1, "schema_version": "visible-map-v1",
        "projection_version": "runtime-map-v1", "game_build": "build",
        "mod_version": "map-sentinel", "map_instance_id": "map-1", "act_id": 1,
        "scope_id": "scope-1", "availability": "available", "completeness": "complete",
        "freshness": "current", "reason": null, "nodes": nodes, "edges": edges,
        "position": {"kind": "current", "node_id": "start"}, "history": ["start"],
        "terminal_node_ids": [node_ids[255]],
        "bindings": [{
            "graph_node_id": node_ids[1],
            "host_action_id": "move-1",
            "action": {"kind": "select_map_node", "node_id": node_ids[1]}
        }]
    });
    let snapshot_digest =
        sts2_harness::sha256_hex(serde_json::to_vec(&snapshot).expect("map snapshot serializes"));
    let request = json!({
        "schema": "sts2.exo-decision-map-v1",
        "provider_revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "model_execution_id": "model-1",
        "state_id": "state-1",
        "generation": 1,
        "observation": {
            "state_id": "state-1",
            "generation": 1,
            "visible_seed": null,
            "player": {
                "hp": 50,
                "max_hp": 50,
                "energy": 3,
                "gold": 99,
                "hand": [],
                "deck": [],
                "discard": [],
                "exhaust": []
            },
            "state": {"state": "map", "node_id": "start", "options": [node_ids[1]]},
            "legal_actions": [{
                "action_id": "move-1",
                "action": {"kind": "select_map_node", "node_id": node_ids[1]}
            }]
        },
        "legal_action_ids": ["move-1"],
        "objective": "choose a legal map node",
        "hard_constraints": [],
        "max_response_bytes": 8192,
        "map_context": {
            "profile": "runtime-map-v1",
            "schema_digest":
                "ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b",
            "snapshot_digest": snapshot_digest,
            "snapshot": snapshot
        }
    });
    serde_json::to_vec(&request).expect("map request serializes")
}
