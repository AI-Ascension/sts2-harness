// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::path::PathBuf;
use sts2_harness::{MapBundleError, MapViewBundle, RuntimeMapBundleIdentity};

fn fixture_snapshot() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/map-bundle-v1");
    let feed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("feed.json")).expect("fixture feed"))
            .expect("fixture feed JSON");
    let directory = root.join(feed["head"].as_str().expect("fixture feed head"));
    std::fs::read(directory.join("visible-map.json")).expect("fixture snapshot")
}

fn identity() -> RuntimeMapBundleIdentity {
    RuntimeMapBundleIdentity {
        run_id: "run".to_owned(),
        episode_id: "episode".to_owned(),
        trajectory_id: "trajectory".to_owned(),
        model_execution_id: None,
        action_catalog_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
    }
}

#[test]
fn runtime_snapshot_builder_uses_protocol_duplicate_key_validation() {
    let snapshot = fixture_snapshot();
    let marker = br#","availability"#;
    let position = snapshot
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("availability marker");
    let mut duplicate = snapshot[..position].to_vec();
    duplicate.extend_from_slice(br#","act_id":1"#);
    duplicate.extend_from_slice(&snapshot[position..]);
    assert!(matches!(
        MapViewBundle::from_runtime_snapshot(duplicate, identity()),
        Err(MapBundleError::ProtocolSnapshot(_))
    ));
}

#[test]
fn runtime_snapshot_builder_preserves_long_host_action_ids() {
    let mut value: serde_json::Value = serde_json::from_slice(&fixture_snapshot()).expect("JSON");
    let bindings = value["bindings"].as_array_mut().expect("bindings");
    bindings[0]["host_action_id"] = serde_json::Value::String("a".repeat(144));
    let snapshot = serde_json::to_vec(&value).expect("snapshot JSON");
    let bundle = MapViewBundle::from_runtime_snapshot(snapshot, identity()).expect("bundle");
    assert_eq!(
        bundle.analysis.legal_destination_counts[0].action_id.len(),
        144
    );
}
