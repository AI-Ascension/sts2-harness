// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BlobDigest, ExactArtifactStore, ExactCheckpointError, ExactCheckpointId, ExactStateDigest,
    plan_retention, sweep,
};

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-retention-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("test workspace is creatable");
    path
}

fn state_digest() -> ExactStateDigest {
    ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "a".repeat(64)))
        .expect("state digest is valid")
}

fn manifest(payload: &BlobDigest, restore: &BlobDigest) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "exact_state_digest": state_digest().as_str(),
        "canonical_payload": {"digest": payload.as_str()},
        "restore_artifacts": [{"digest": restore.as_str()}],
    }))
    .expect("manifest serializes")
}

#[test]
fn shared_dependency_survives_while_any_manifest_is_pinned() {
    let store = ExactArtifactStore::new(workspace("shared"));
    let shared = store.stage_blob(b"shared").expect("shared stages");
    let left = store.stage_blob(b"left").expect("left stages");
    let right = store.stage_blob(b"right").expect("right stages");
    let pinned = store
        .publish_manifest(&manifest(&shared, &left))
        .expect("left manifest publishes");
    store
        .publish_manifest(&manifest(&shared, &right))
        .expect("right manifest publishes");

    let plan = plan_retention(&store, std::slice::from_ref(&pinned)).expect("plan builds");
    let retained: Vec<&str> = plan.retained_blobs.iter().map(BlobDigest::as_str).collect();
    assert!(retained.contains(&shared.as_str()));
    assert!(retained.contains(&left.as_str()));
    assert!(plan.collectable_blobs.iter().any(|d| d == &right));
    assert!(plan.missing_blobs.is_empty());

    let removed = sweep(&store, &plan).expect("sweep succeeds");
    assert_eq!(removed, 1);
    assert!(store.read_blob(&shared).is_ok());
    assert!(store.read_blob(&left).is_ok());
    assert_eq!(
        store.read_blob(&right).expect_err("collectable removed"),
        ExactCheckpointError::Missing
    );
    assert!(
        store
            .stored_manifests()
            .expect("manifests list")
            .contains(&pinned)
    );
}

#[test]
fn missing_pinned_dependency_fails_closed() {
    let store = ExactArtifactStore::new(workspace("missing"));
    let payload = store.stage_blob(b"payload").expect("payload stages");
    let absent = BlobDigest::parse(&format!("sha256:{}", "e".repeat(64))).expect("digest parses");
    let pinned = store
        .publish_manifest(&manifest(&payload, &absent))
        .expect("manifest publishes");

    let plan = plan_retention(&store, &[pinned]).expect("plan builds");
    assert_eq!(plan.missing_blobs, vec![absent]);
    assert_eq!(
        sweep(&store, &plan).expect_err("sweep refuses"),
        ExactCheckpointError::Missing
    );
    assert!(store.read_blob(&payload).is_ok());
}

#[test]
fn stale_staging_files_are_recovered() {
    let store = ExactArtifactStore::new(workspace("recover"));
    let digest = store.stage_blob(b"staged").expect("blob stages");
    let hex = digest.as_str().trim_start_matches("sha256:");
    let directory = store
        .root_directory()
        .join("exact")
        .join("blobs")
        .join(&hex[..2]);
    fs::create_dir_all(&directory).expect("prefix directory exists");
    let staged = directory.join(".tmp-9999");
    fs::write(&staged, b"partial").expect("partial staging file writes");

    assert_eq!(store.recover_temporaries().expect("recovery runs"), 1);
    assert!(!staged.exists());
    assert!(store.read_blob(&digest).is_ok());
}

#[test]
fn stored_manifests_are_ordered_and_parseable() {
    let store = ExactArtifactStore::new(workspace("list"));
    let payload = store.stage_blob(b"p").expect("payload stages");
    let other = store.stage_blob(b"q").expect("other blob stages");
    let first = store
        .publish_manifest(&manifest(&payload, &payload))
        .expect("first publishes");
    let second = store
        .publish_manifest(&manifest(&other, &payload))
        .expect("second publishes");
    let mut expected = vec![first, second.clone()];
    expected.sort();
    let listed = store.stored_manifests().expect("manifests list");
    assert_eq!(listed, expected);
    assert!(ExactCheckpointId::parse(second.as_str()).is_ok());
}
