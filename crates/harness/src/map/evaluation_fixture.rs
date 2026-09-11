// SPDX-License-Identifier: MIT

use serde_json::json;

pub(super) fn matrix_snapshot() -> Vec<u8> {
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
