// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BlobDigest, ExactArtifactStore, ExactAssurance, ExactCheckpointError, ExactCheckpointId,
    ExactCheckpointReference, ExactStateDigest, MAX_EXACT_BLOB_BYTES,
};

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-exact-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("test workspace is creatable");
    path
}

fn state_digest() -> ExactStateDigest {
    ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "a".repeat(64)))
        .expect("state digest is valid")
}

fn reference(identifier: ExactCheckpointId, digest: ExactStateDigest) -> ExactCheckpointReference {
    ExactCheckpointReference {
        exact_state_digest: digest,
        exact_checkpoint_id: identifier,
        boundary_kind: "decision".to_owned(),
        boundary_phase: "COMBAT".to_owned(),
        assurance: ExactAssurance::CaptureOnly,
    }
}

fn manifest_bytes(digest: &ExactStateDigest) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "exact_state_digest": digest.as_str(),
    }))
    .expect("manifest serializes")
}

#[test]
fn staged_blobs_round_trip_by_content_identity() {
    let store = ExactArtifactStore::new(workspace("round-trip"));
    let digest = store
        .stage_blob(b"exact-state-payload")
        .expect("stage succeeds");
    assert!(digest.as_str().starts_with("sha256:"));
    let bytes = store.read_blob(&digest).expect("read succeeds");
    assert_eq!(bytes, b"exact-state-payload");
}

#[test]
fn identical_content_is_deduplicated() {
    let store = ExactArtifactStore::new(workspace("dedup"));
    let first = store.stage_blob(b"same").expect("first stage");
    let second = store.stage_blob(b"same").expect("second stage");
    assert_eq!(first, second);
    assert!(store.read_blob(&first).is_ok());
}

#[test]
fn tampered_or_missing_blobs_are_rejected() {
    let root = workspace("tamper");
    let store = ExactArtifactStore::new(&root);
    let digest = store.stage_blob(b"original").expect("stage succeeds");
    let hex = digest.as_str().trim_start_matches("sha256:");
    let path = root.join("exact").join("blobs").join(&hex[..2]).join(hex);
    fs::write(&path, b"tampered").expect("test tamper write");
    assert_eq!(
        store.read_blob(&digest).expect_err("tamper detected"),
        ExactCheckpointError::DigestMismatch
    );
    let missing = BlobDigest::parse(&format!("sha256:{}", "b".repeat(64))).expect("digest parses");
    assert_eq!(
        store.read_blob(&missing).expect_err("missing detected"),
        ExactCheckpointError::Missing
    );
}

#[test]
fn identity_namespaces_are_enforced() {
    let state = state_digest().as_str().to_owned();
    assert!(ExactCheckpointId::parse(&state).is_err());
    assert!(BlobDigest::parse(&state).is_err());
    assert!(BlobDigest::parse("sha256:../../etc/passwd").is_err());
    assert!(BlobDigest::parse(&format!("sha256:{}", "A".repeat(64))).is_err());
    assert!(BlobDigest::parse("sha256:00").is_err());
    assert!(ExactStateDigest::parse(&state).is_ok());
}

#[test]
fn oversized_blobs_are_refused_before_storage() {
    let store = ExactArtifactStore::new(workspace("oversize"));
    let oversized = vec![0_u8; MAX_EXACT_BLOB_BYTES + 1];
    assert_eq!(
        store.stage_blob(&oversized).expect_err("oversize refused"),
        ExactCheckpointError::Oversized
    );
}

#[test]
fn manifests_publish_and_bind_their_reference() {
    let store = ExactArtifactStore::new(workspace("manifest"));
    let digest = state_digest();
    let identifier = store
        .publish_manifest(&manifest_bytes(&digest))
        .expect("manifest publishes");
    assert!(identifier.as_str().starts_with("asc-checkpoint:v1:sha256:"));
    let captured = reference(identifier.clone(), digest);
    store
        .verify_reference(&captured)
        .expect("reference verifies");

    let other = ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "c".repeat(64)))
        .expect("digest is valid");
    let mismatched = reference(identifier, other);
    assert_eq!(
        store
            .verify_reference(&mismatched)
            .expect_err("mismatch rejected"),
        ExactCheckpointError::DigestMismatch
    );
}

#[test]
fn missing_manifest_and_invalid_boundary_are_rejected() {
    let store = ExactArtifactStore::new(workspace("missing-manifest"));
    let identifier =
        ExactCheckpointId::parse(&format!("asc-checkpoint:v1:sha256:{}", "d".repeat(64)))
            .expect("identifier is valid");
    assert_eq!(
        store.read_manifest(&identifier).expect_err("missing"),
        ExactCheckpointError::Missing
    );
    let mut reference = reference(identifier, state_digest());
    reference.boundary_phase = String::new();
    assert_eq!(
        store.verify_reference(&reference).expect_err("boundary"),
        ExactCheckpointError::InvalidBoundary
    );
}

#[test]
fn assurance_levels_stay_distinct() {
    assert!(!ExactAssurance::PublicObservationOnly.is_exact());
    assert!(ExactAssurance::CaptureOnly.is_exact());
    assert!(ExactAssurance::RestoreSupported.is_exact());
    assert!(ExactAssurance::RestoreVerified.is_exact());
    assert!(ExactAssurance::ContinuationCertified.is_exact());
    let labels = [
        ExactAssurance::PublicObservationOnly,
        ExactAssurance::CaptureOnly,
        ExactAssurance::RestoreSupported,
        ExactAssurance::RestoreVerified,
        ExactAssurance::ContinuationCertified,
    ]
    .map(ExactAssurance::as_str);
    let unique: std::collections::BTreeSet<&str> = labels.into_iter().collect();
    assert_eq!(unique.len(), 5);
}
