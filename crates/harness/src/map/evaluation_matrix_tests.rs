// SPDX-License-Identifier: MIT

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

    #[test]
    fn adapter_memoizes_dense_layered_routes_with_a_finite_bound() {
        let layer_count = 80_usize;
        let mut edges = vec![serde_json::json!({
            "from": "start",
            "to": "n0:left"
        })];
        edges.push(serde_json::json!({
            "from": "start",
            "to": "n0:right"
        }));
        for layer in 0..layer_count.saturating_sub(1) {
            for side in ["left", "right"] {
                for next_side in ["left", "right"] {
                    edges.push(serde_json::json!({
                        "from": format!("n{layer}:{side}"),
                        "to": format!("n{}:{next_side}", layer + 1)
                    }));
                }
            }
        }
        let context = serde_json::json!({
            "graph": {
                "position": {"node_id":"start"},
                "edges": edges,
                "terminal_node_ids": [
                    format!("n{}:left", layer_count - 1),
                    format!("n{}:right", layer_count - 1)
                ],
                "bindings": [{
                    "graph_node_id":"n0:left",
                    "host_action_id":"select-map-node:n0:left"
                }]
            }
        });
        let bytes = serde_json::to_vec(&context).expect("dense context");
        let (_, route) = proposed_decision(&bytes).expect("dense route");
        assert_eq!(route.len(), layer_count + 1);
    }
}
