// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BlobDigest, ExactArtifactStore, ExactAssurance, ExactCheckpointId, ExactCheckpointReference,
    ExactStateDigest, VerificationFailure, VerificationOutcome, verify_checkpoint,
};

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-verify-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("test workspace is creatable");
    path
}

fn blob(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn state(seed: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("state digest")
}

fn unpublished_id() -> ExactCheckpointId {
    ExactCheckpointId::parse(&format!("asc-checkpoint:v1:sha256:{}", "b".repeat(64)))
        .expect("checkpoint id")
}

fn reference(state_seed: char, checkpoint_id: ExactCheckpointId) -> ExactCheckpointReference {
    ExactCheckpointReference {
        exact_state_digest: state(state_seed),
        exact_checkpoint_id: checkpoint_id,
        boundary_kind: "decision".to_owned(),
        boundary_phase: "COMBAT".to_owned(),
        assurance: ExactAssurance::CaptureOnly,
    }
}

fn manifest_bytes(state_seed: char, payload: &BlobDigest, restore: &BlobDigest) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "exact_state_digest": state(state_seed).as_str(),
        "canonical_payload": {"digest": payload.as_str()},
        "restore_artifacts": [{"digest": restore.as_str()}],
        "compatibility_digest": blob('c'),
        "coverage_contract_digest": blob('d'),
    }))
    .expect("manifest serializes")
}

fn publish(
    store: &ExactArtifactStore,
    state_seed: char,
    payload: &BlobDigest,
    restore: &BlobDigest,
) -> ExactCheckpointId {
    store
        .publish_manifest(&manifest_bytes(state_seed, payload, restore))
        .expect("manifest publishes")
}

#[test]
fn verification_establishes_integrity_only() {
    let store = ExactArtifactStore::new(workspace("happy"));
    let payload = store.stage_blob(b"payload").expect("payload stages");
    let restore = store.stage_blob(b"restore").expect("restore stages");
    let identifier = publish(&store, 'a', &payload, &restore);
    let start = reference('a', identifier);

    match verify_checkpoint(&store, &start, &blob('c'), &blob('d')) {
        VerificationOutcome::Verified(evidence) => {
            assert!(evidence.integrity_verified);
            assert!(!evidence.restore_verified);
            assert!(!evidence.continuation_certified);
            assert_eq!(evidence.highest_level(), "integrity_verified");
            assert_eq!(evidence.producer, "harness:verify");
        }
        other => panic!("expected verification, got {other:?}"),
    }
}

#[test]
fn mismatched_contracts_and_digests_are_reported_separately() {
    let store = ExactArtifactStore::new(workspace("mismatch"));
    let payload = store.stage_blob(b"payload").expect("payload stages");
    let identifier = publish(&store, 'a', &payload, &payload);
    let start = reference('a', identifier.clone());

    assert_eq!(
        verify_checkpoint(&store, &start, &blob('e'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::IncompatibleProfile)
    );
    assert_eq!(
        verify_checkpoint(&store, &start, &blob('c'), &blob('e')),
        VerificationOutcome::Rejected(VerificationFailure::CoverageIncomplete)
    );

    let other_state = reference('f', identifier);
    assert_eq!(
        verify_checkpoint(&store, &other_state, &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure)
    );
}

#[test]
fn missing_manifest_and_missing_dependency_are_distinguished() {
    let store = ExactArtifactStore::new(workspace("missing"));
    let absent = reference('a', unpublished_id());
    assert_eq!(
        verify_checkpoint(&store, &absent, &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::ManifestMissing)
    );

    let present = store.stage_blob(b"present").expect("blob stages");
    let absent_blob = BlobDigest::parse(&blob('e')).expect("digest parses");
    let identifier = publish(&store, 'a', &present, &absent_blob);
    let start = reference('a', identifier);
    assert_eq!(
        verify_checkpoint(&store, &start, &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::MissingDependency)
    );
}

#[test]
fn malformed_and_tampered_manifests_are_rejected() {
    let store = ExactArtifactStore::new(workspace("tamper"));
    let payload = store.stage_blob(b"payload").expect("payload stages");
    let identifier = publish(&store, 'a', &payload, &payload);
    let start = reference('a', identifier.clone());
    let hex = identifier
        .as_str()
        .trim_start_matches("asc-checkpoint:v1:sha256:");
    let path = store
        .root_directory()
        .join("exact")
        .join("manifests")
        .join(&hex[..2])
        .join(hex);
    fs::write(&path, b"not-json").expect("tamper write");
    assert_eq!(
        verify_checkpoint(&store, &start, &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::MalformedManifest)
    );
}

#[test]
fn tampered_payload_bytes_are_rejected() {
    let store = ExactArtifactStore::new(workspace("blob-tamper"));
    let payload = store.stage_blob(b"payload").expect("payload stages");
    let identifier = publish(&store, 'a', &payload, &payload);
    let start = reference('a', identifier);
    let hex = payload.as_str().trim_start_matches("sha256:");
    let path = store
        .root_directory()
        .join("exact")
        .join("blobs")
        .join(&hex[..2])
        .join(hex);
    fs::write(&path, b"tampered").expect("blob tamper");
    assert_eq!(
        verify_checkpoint(&store, &start, &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure)
    );
}
