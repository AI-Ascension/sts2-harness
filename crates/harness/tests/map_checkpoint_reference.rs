// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use sts2_harness::{
    BundleFileStore, MapBundleError, MapBundleFeed, MapViewBundle, PublicCheckpointSummary,
};

fn fixture() -> MapViewBundle {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/map-bundle-v1");
    BundleFileStore::new(root).unwrap().head().unwrap().unwrap()
}

// Synthetic public-envelope fixture, generated locally; no game or privileged input is used.
fn reference() -> PublicCheckpointSummary {
    PublicCheckpointSummary {
        schema: "ascension.exact_checkpoint_reference.v1".into(),
        reference_version: "exact-checkpoint-reference-v1".into(),
        handle: format!("ckpt-h1:{}", "a".repeat(64)),
        occurrence: "map:occurrence:one".into(),
        boundary_kind: "decision".into(),
        boundary_phase: "MAP".into(),
        assurance: "capture_only".into(),
        restore_verified: false,
    }
}

fn workspace() -> PathBuf {
    let root = std::env::temp_dir().join(format!("map-checkpoint-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn schema(name: &str) -> jsonschema::Validator {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let value: Value = serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap();
    jsonschema::validator_for(&value).unwrap()
}

#[test]
fn legacy_bundle_remains_byte_identical_and_referenced_v2_has_distinct_identity() {
    let legacy = fixture();
    let before = legacy.canonical_manifest_bytes().unwrap();
    assert!(
        !serde_json::from_slice::<Value>(&before)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("checkpoint_reference")
    );
    let newer = legacy
        .clone()
        .with_checkpoint_reference(reference())
        .unwrap();
    assert_eq!(legacy.canonical_manifest_bytes().unwrap(), before);
    assert_eq!(newer.manifest.bundle_version, "sts2.map-view-bundle-v2");
    assert_ne!(newer.bundle_digest(), legacy.bundle_digest());
    assert_eq!(newer.snapshot_bytes, legacy.snapshot_bytes);
    assert_eq!(newer.analysis, legacy.analysis);
    assert_eq!(newer.manifest.checkpoint_reference, Some(reference()));
    let validator = schema("map-view-bundle.schema.json");
    assert!(validator.is_valid(&serde_json::from_slice::<Value>(&before).unwrap()));
    assert!(validator.is_valid(&serde_json::to_value(&newer.manifest).unwrap()));
}

#[test]
fn publishing_mixed_history_preserves_references_and_supports_idempotent_retry() {
    let root = workspace();
    let store = BundleFileStore::new(&root).unwrap();
    let legacy = fixture();
    store.publish_at(&legacy, "legacy", 1).unwrap();
    assert_eq!(store.read_feed().unwrap().feed_version, "sts2.map-feed-v1");
    let newer = legacy.with_checkpoint_reference(reference()).unwrap();
    store.publish_at(&newer, "checkpoint", 2).unwrap();
    let retry = store.publish_at(&newer, "retry", 3).unwrap();
    assert!(retry.already_present());
    let feed = store.read_feed().unwrap();
    assert_eq!(feed.feed_version, "sts2.map-feed-v2");
    assert_eq!(feed.entries.len(), 2);
    assert!(feed.entries[0].checkpoint_reference.is_none());
    assert_eq!(feed.entries[1].checkpoint_reference, Some(reference()));
    assert_eq!(store.head().unwrap().unwrap(), newer);
    assert_eq!(store.load(newer.bundle_digest()).unwrap(), newer);
    assert!(schema("map-feed.schema.json").is_valid(&serde_json::to_value(feed).unwrap()));
}

#[test]
fn feed_bundle_reference_mismatch_or_omission_is_rejected_before_head_exposure() {
    for replacement in [None, Some(json!({"occurrence": "another"}))] {
        let root = workspace();
        let store = BundleFileStore::new(&root).unwrap();
        let bundle = fixture().with_checkpoint_reference(reference()).unwrap();
        store.publish_at(&bundle, "publish", 1).unwrap();
        let mut feed: Value =
            serde_json::from_slice(&fs::read(root.join("feed.json")).unwrap()).unwrap();
        match replacement {
            None => {
                feed["entries"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("checkpoint_reference");
            }
            Some(value) => {
                feed["entries"][0]["checkpoint_reference"]["occurrence"] =
                    value["occurrence"].clone()
            }
        }
        fs::write(root.join("feed.json"), serde_json::to_vec(&feed).unwrap()).unwrap();
        assert_eq!(store.head().unwrap_err(), MapBundleError::IdentityMismatch);
        assert_eq!(
            store.load(bundle.bundle_digest()).unwrap_err(),
            MapBundleError::IdentityMismatch
        );
        assert_eq!(
            store.publish_at(&bundle, "retry", 2).unwrap_err(),
            MapBundleError::IdentityMismatch
        );
    }
}

#[test]
fn closed_versions_and_public_reference_validation_agree_with_schema() {
    let legacy = serde_json::to_value(fixture().manifest).unwrap();
    let mut valid = legacy.clone();
    valid["bundle_version"] = "sts2.map-view-bundle-v2".into();
    valid["checkpoint_reference"] = serde_json::to_value(reference()).unwrap();
    let validator = schema("map-view-bundle.schema.json");
    let mut invalid = Vec::new();
    let mut item = legacy.clone();
    item["checkpoint_reference"] = serde_json::to_value(reference()).unwrap();
    invalid.push(item);
    let mut item = legacy.clone();
    item["checkpoint_reference"] = Value::Null;
    invalid.push(item);
    let mut item = legacy;
    item["bundle_version"] = "sts2.map-view-bundle-v2".into();
    invalid.push(item);
    for (field, value) in [
        ("exact_state_digest", json!("privileged")),
        ("reference_version", json!("future")),
        ("handle", json!("sha256:raw")),
        ("restore_verified", json!(true)),
    ] {
        let mut item = valid.clone();
        item["checkpoint_reference"][field] = value;
        invalid.push(item);
    }
    for item in invalid {
        assert!(!validator.is_valid(&item), "schema accepted {item}");
        assert!(
            MapViewBundle::decode_manifest(&serde_json::to_vec(&item).unwrap()).is_err(),
            "decoder accepted {item}"
        );
    }
}

#[test]
fn v1_feed_rejects_reference_injection_and_null_references_are_not_absence() {
    let root = workspace();
    let store = BundleFileStore::new(&root).unwrap();
    store.publish_at(&fixture(), "legacy", 1).unwrap();
    let original: Value =
        serde_json::from_slice(&fs::read(root.join("feed.json")).unwrap()).unwrap();
    let validator = schema("map-feed.schema.json");
    for reference in [serde_json::to_value(reference()).unwrap(), Value::Null] {
        let mut feed = original.clone();
        feed["entries"][0]["checkpoint_reference"] = reference;
        assert!(!validator.is_valid(&feed));
        fs::write(root.join("feed.json"), serde_json::to_vec(&feed).unwrap()).unwrap();
        assert!(store.read_feed().is_err());
    }
}
