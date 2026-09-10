// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::episode::legal_actions::{ActionKind, EpisodeLegalAction};
use serde_json::json;

fn snapshot() -> Value {
    json!({
        "state_id":"state-1", "generation":1, "schema_version":"visible-map-v1",
        "projection_version":"runtime-map-v1", "game_build":"build", "mod_version":"mod",
        "map_instance_id":"map-1", "act_id":1, "scope_id":"scope-1", "availability":"available",
        "completeness":"complete", "freshness":"current", "reason":null,
        "nodes":[
            {"id":"a-start","row":0,"column":0,"category":"start","visited":true},
            {"id":"b-next","row":1,"column":0,"category":"monster","visited":false}
        ],
        "edges":[{"from":"a-start","to":"b-next"}],
        "position":{"kind":"current","node_id":"a-start"}, "history":["a-start"],
        "terminal_node_ids":["b-next"],
        "bindings":[{"graph_node_id":"b-next","host_action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":"option-1"}}]
    })
}

fn wrapper(snapshot: &Value) -> Value {
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(snapshot).unwrap())
    );
    json!({
        "profile":RUNTIME_MAP_PROFILE,
        "schema_digest":RUNTIME_MAP_SCHEMA_DIGEST,
        "snapshot_digest":digest,
        "snapshot":snapshot
    })
}

fn actions() -> EpisodeLegalActionSet {
    EpisodeLegalActionSet::new(
        "state-1",
        1,
        vec![EpisodeLegalAction::new("move-1", ActionKind::SelectMapNode).unwrap()],
    )
    .unwrap()
}

fn canonical_response() -> Value {
    json!({
        "correlation_id":"corr-42", "generation":42, "instance_id":"instance-1",
        "kind":"snapshot_response", "lease_epoch":7, "lease_id":"lease-1",
        "protocol_version":RUNTIME_MAP_PROFILE,
        "provenance":{"artifact":"sts2-protocol/runtime-map-v1",
            "generator":"hand-authored", "source":"schemas/runtime-map-v1.schema.json"},
        "schema_digest":RUNTIME_MAP_SCHEMA_DIGEST, "session_id":"session-1",
        "snapshot":{
            "act_id":1, "availability":"available",
            "bindings":[
                {"action":{"kind":"select_map_node","node_id":"map-option:42:left"},
                    "graph_node_id":"map:1:1:0",
                    "host_action_id":"select-map-node:42:map:1:1:0"},
                {"action":{"kind":"select_map_node","node_id":"map-option:42:right"},
                    "graph_node_id":"map:1:1:1",
                    "host_action_id":"select-map-node:42:map:1:1:1"}
            ],
            "completeness":"complete",
            "edges":[
                {"from":"map:1:0:0","to":"map:1:1:0"},
                {"from":"map:1:0:0","to":"map:1:1:1"},
                {"from":"map:1:1:0","to":"map:1:2:0"},
                {"from":"map:1:1:1","to":"map:1:2:0"}
            ],
            "freshness":"current", "game_build":"0.103.2", "generation":42,
            "history":["map:1:0:0"], "map_instance_id":"map-instance-1",
            "mod_version":"map-mod-1",
            "nodes":[
                {"category":"start","column":0,"id":"map:1:0:0","row":0,"visited":true},
                {"category":"monster","column":0,"id":"map:1:1:0","row":1,"visited":false},
                {"category":"event","column":1,"id":"map:1:1:1","row":1,"visited":false},
                {"category":"boss","column":0,"id":"map:1:2:0","row":2,"visited":false}
            ],
            "position":{"kind":"current","node_id":"map:1:0:0"},
            "projection_version":RUNTIME_MAP_PROFILE, "reason":null,
            "schema_version":"visible-map-v1", "scope_id":"campaign-1",
            "state_id":"map-state-42", "terminal_node_ids":["map:1:2:0"]
        },
        "timeout":{"elapsed_millis":12,"timeout_millis":1000}
    })
}

fn canonical_actions() -> EpisodeLegalActionSet {
    EpisodeLegalActionSet::new(
        "map-state-42",
        42,
        vec![
            EpisodeLegalAction::new("select-map-node:42:map:1:1:0", ActionKind::SelectMapNode)
                .unwrap(),
            EpisodeLegalAction::new("select-map-node:42:map:1:1:1", ActionKind::SelectMapNode)
                .unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn wrapper_rejects_unknown_fields_and_digest_or_payload_tampering() {
    let snapshot = snapshot();
    let value = wrapper(&snapshot);
    let parsed = MapDecisionContext::from_exo_value(&value, "state-1", 1, &["move-1".into()]);
    assert!(parsed.is_ok());

    let mut unknown = value.clone();
    unknown["extra"] = json!(true);
    assert_eq!(
        MapDecisionContext::from_exo_value(&unknown, "state-1", 1, &["move-1".into()]),
        Err(MapError::InvalidContext)
    );

    let mut bad_digest = value.clone();
    bad_digest["snapshot_digest"] = json!("0");
    assert_eq!(
        MapDecisionContext::from_exo_value(&bad_digest, "state-1", 1, &["move-1".into()]),
        Err(MapError::InvalidDigest)
    );

    let mut bad_snapshot = value.clone();
    bad_snapshot["snapshot"]["game_build"] = json!("tampered");
    assert_eq!(
        MapDecisionContext::from_exo_value(&bad_snapshot, "state-1", 1, &["move-1".into()]),
        Err(MapError::InvalidDigest)
    );
}

#[test]
fn canonical_runtime_map_golden_reaches_the_consumer_parser() {
    let value = canonical_response();
    let actions = canonical_actions();
    let context = MapDecisionContext::from_mcp_value(&value, "map-state-42", 42, &actions)
        .expect("canonical runtime-map golden must parse");
    assert_eq!(
        context.snapshot_digest(),
        "49a937716806388f25d2a5536f2b7be7a31a42b42e436041a033bef176688194"
    );
}

#[test]
fn equivalent_map_collection_order_uses_one_canonical_digest() {
    let canonical = canonical_response();
    let mut reordered = canonical.clone();
    reordered["snapshot"]["nodes"]
        .as_array_mut()
        .unwrap()
        .reverse();
    reordered["snapshot"]["edges"]
        .as_array_mut()
        .unwrap()
        .reverse();
    reordered["snapshot"]["bindings"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let first =
        MapDecisionContext::from_mcp_value(&canonical, "map-state-42", 42, &canonical_actions())
            .unwrap();
    let second =
        MapDecisionContext::from_mcp_value(&reordered, "map-state-42", 42, &canonical_actions())
            .unwrap();
    assert_eq!(first.snapshot_digest(), second.snapshot_digest());
}

#[test]
fn wrapper_rejects_stale_identity_generation_and_catalog() {
    let value = wrapper(&snapshot());
    assert_eq!(
        MapDecisionContext::from_exo_value(&value, "other-state", 1, &["move-1".into()]),
        Err(MapError::CatalogMismatch)
    );
    assert_eq!(
        MapDecisionContext::from_exo_value(&value, "state-1", 2, &["move-1".into()]),
        Err(MapError::CatalogMismatch)
    );
    assert_eq!(
        MapDecisionContext::from_exo_value(&value, "state-1", 1, &["other-action".into()]),
        Err(MapError::CatalogMismatch)
    );
}

#[test]
fn mcp_envelope_checks_identity_generation_timeout_and_unknown_fields() {
    let envelope = json!({
        "protocol_version": RUNTIME_MAP_PROFILE,
        "schema_digest": RUNTIME_MAP_SCHEMA_DIGEST,
        "provenance": {"artifact":"sts2-protocol/runtime-map-v1",
            "source":"schemas/runtime-map-v1.schema.json", "generator":"hand-authored"},
        "correlation_id":"3", "instance_id":"instance-1", "session_id":"session-1",
        "lease_id":"lease-1", "lease_epoch":7, "generation":1,
        "kind":"snapshot_response", "snapshot":snapshot(),
        "timeout":{"timeout_millis":1000,"elapsed_millis":12}
    });
    assert!(MapDecisionContext::from_mcp_value(&envelope, "state-1", 1, &actions()).is_ok());
    for mutation in [
        json!({"schema_digest":"0"}),
        json!({"generation":2}),
        json!({"timeout":{"timeout_millis":1,"elapsed_millis":2}}),
        json!({"extra":true}),
    ] {
        let mut invalid = envelope.clone();
        for (key, value) in mutation.as_object().unwrap() {
            invalid[key] = value.clone();
        }
        assert!(MapDecisionContext::from_mcp_value(&invalid, "state-1", 1, &actions()).is_err());
    }
}

#[test]
fn mcp_envelope_rejects_foreign_identity_and_collection_overflow() {
    let mut foreign = canonical_response();
    foreign["lease_id"] = json!("lease with spaces");
    assert_eq!(
        MapDecisionContext::from_mcp_value(&foreign, "map-state-42", 42, &canonical_actions()),
        Err(MapError::InvalidIdentity)
    );

    let actions = canonical_actions();
    let mut too_many_nodes = canonical_response();
    too_many_nodes["snapshot"]["nodes"]
        .as_array_mut()
        .unwrap()
        .extend((0..253).map(|index| {
            json!({"id":format!("extra-{index}"),"row":index,"column":0,
                "category":"other","visited":false})
        }));
    assert_eq!(
        MapDecisionContext::from_mcp_value(&too_many_nodes, "map-state-42", 42, &actions),
        Err(MapError::InvalidNode)
    );

    let mut too_many_edges = canonical_response();
    too_many_edges["snapshot"]["edges"]
        .as_array_mut()
        .unwrap()
        .resize(1_025, json!({"from":"map:1:0:0","to":"map:1:1:0"}));
    assert_eq!(
        MapDecisionContext::from_mcp_value(&too_many_edges, "map-state-42", 42, &actions),
        Err(MapError::InvalidEdge)
    );

    let mut too_many_bindings = canonical_response();
    too_many_bindings["snapshot"]["bindings"]
        .as_array_mut()
        .unwrap()
        .resize(
            257,
            json!({"graph_node_id":"map:1:1:0","host_action_id":"move",
                "action":{"kind":"select_map_node","node_id":"option"}}),
        );
    assert_eq!(
        MapDecisionContext::from_mcp_value(&too_many_bindings, "map-state-42", 42, &actions),
        Err(MapError::InvalidBinding)
    );
}
