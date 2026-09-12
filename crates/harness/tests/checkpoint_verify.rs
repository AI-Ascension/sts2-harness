// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    BlobDigest, ExactArtifactStore, ExactAssurance, ExactCheckpointId, ExactCheckpointReference,
    ExactStateDigest, VerificationFailure, VerificationOutcome, verify_checkpoint,
};

const PAYLOAD: &[u8] =
    include_bytes!("../../../protocol-artifact/exact-state-v1/golden/state.canonical");

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
    if seed == 'c' {
        return "sha256:95c56e287c1e000c70b33fc0180caf7c58f1da0606681e18a67c43bfc6ab8974"
            .to_owned();
    }
    if seed == 'd' {
        return "sha256:2fa9e73895d97193dc0153eae157f93da76c3292a526a4bbe2486e755e93b82b"
            .to_owned();
    }
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn state(seed: char) -> ExactStateDigest {
    if seed == 'a' {
        let mut hash = Sha256::new();
        hash.update(b"AI-ASCENSION/EXACT-STATE/v1\0");
        hash.update(PAYLOAD);
        let hex: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let digest = format!("asc-state:v1:sha256:{hex}");
        return ExactStateDigest::parse(&digest).expect("synthetic payload digest");
    }
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
        boundary_phase: "CONTRACT_FIXTURE".to_owned(),
        assurance: ExactAssurance::CaptureOnly,
    }
}

fn manifest_bytes(state_seed: char, payload: &BlobDigest, restore: &BlobDigest) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "canonical_profile": "asc-jcs-state-v1",
        "boundary": {"kind":"decision", "phase":"CONTRACT_FIXTURE"},
        "origin": {"run_id":"synthetic", "generation":0},
        "parent_checkpoint_id": null,
        "exact_state_digest": state(state_seed).as_str(),
        "canonical_payload": {"digest": payload.as_str(), "role":"exact_state_payload", "codec":"asc-jcs-state-v1", "size_bytes":PAYLOAD.len()},
        "restore_artifacts": [{"digest": restore.as_str(), "role":"snapshot", "codec":"test-raw-v1", "size_bytes":if restore == payload { PAYLOAD.len() } else { 7 }}],
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
    let payload = store.stage_blob(PAYLOAD).expect("payload stages");
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
    let payload = store.stage_blob(PAYLOAD).expect("payload stages");
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

    let present = store.stage_blob(PAYLOAD).expect("blob stages");
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
    let payload = store.stage_blob(PAYLOAD).expect("payload stages");
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
    let payload = store.stage_blob(PAYLOAD).expect("payload stages");
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

#[test]
fn incomplete_manifests_cannot_acquire_integrity_evidence() {
    let store = ExactArtifactStore::new(workspace("incomplete"));
    let payload = store.stage_blob(PAYLOAD).expect("stage");
    let valid: serde_json::Value =
        serde_json::from_slice(&manifest_bytes('a', &payload, &payload)).expect("json");
    for field in [
        "canonical_payload",
        "restore_artifacts",
        "schema",
        "canonical_profile",
        "origin",
        "boundary",
    ] {
        let mut invalid = valid.clone();
        invalid.as_object_mut().expect("object").remove(field);
        let id = store
            .publish_manifest(&serde_json::to_vec(&invalid).expect("json"))
            .expect("publish");
        assert_eq!(
            verify_checkpoint(&store, &reference('a', id), &blob('c'), &blob('d')),
            VerificationOutcome::Rejected(VerificationFailure::MalformedManifest),
            "missing {field}"
        );
    }
    for (field, value) in [
        ("restore_artifacts", serde_json::json!([])),
        ("schema", serde_json::json!("future")),
        ("privileged_extra", serde_json::json!(true)),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        let id = store
            .publish_manifest(&serde_json::to_vec(&invalid).expect("json"))
            .expect("publish");
        assert_eq!(
            verify_checkpoint(&store, &reference('a', id), &blob('c'), &blob('d')),
            VerificationOutcome::Rejected(VerificationFailure::MalformedManifest)
        );
    }
}

#[test]
fn payload_identity_and_declared_lengths_are_verified() {
    let store = ExactArtifactStore::new(workspace("claimed-identity"));
    let payload = store.stage_blob(PAYLOAD).expect("stage");
    let false_state = publish(&store, 'f', &payload, &payload);
    assert_eq!(
        verify_checkpoint(&store, &reference('f', false_state), &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure)
    );
    for field in ["canonical_payload", "restore_artifacts"] {
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&manifest_bytes('a', &payload, &payload)).expect("json");
        if field == "canonical_payload" {
            manifest[field]["size_bytes"] = 8.into();
        } else {
            manifest[field][0]["size_bytes"] = 8.into();
        }
        let id = store
            .publish_manifest(&serde_json::to_vec(&manifest).expect("json"))
            .expect("publish");
        assert_eq!(
            verify_checkpoint(&store, &reference('a', id), &blob('c'), &blob('d')),
            VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure)
        );
    }
}

#[test]
fn self_consistent_hashes_do_not_admit_noncanonical_or_invalid_payloads() {
    let store = ExactArtifactStore::new(workspace("payload-shape"));
    let text = std::str::from_utf8(PAYLOAD).expect("utf8");
    let mut floating: serde_json::Value = serde_json::from_slice(PAYLOAD).expect("json");
    floating["state"]["gameplay"]["float"] = serde_json::json!(1.0);
    let mut unknown = floating.clone();
    unknown["schema"] = "future".into();
    for bytes in [
        b"payload".to_vec(),
        format!("{text}\n").into_bytes(),
        text.replace(
            "\"kind\":\"decision\"",
            "\"kind\":\"decision\",\"kind\":\"decision\"",
        )
        .into_bytes(),
        serde_json::to_vec(&floating).expect("json"),
        serde_json::to_vec(&unknown).expect("json"),
    ] {
        let payload = store.stage_blob(&bytes).expect("stage");
        let mut hash = Sha256::new();
        hash.update(b"AI-ASCENSION/EXACT-STATE/v1\0");
        hash.update(&bytes);
        let hex: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let digest =
            ExactStateDigest::parse(&format!("asc-state:v1:sha256:{hex}")).expect("digest");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&manifest_bytes('a', &payload, &payload)).expect("json");
        manifest["exact_state_digest"] = digest.as_str().into();
        manifest["canonical_payload"]["size_bytes"] = bytes.len().into();
        manifest["restore_artifacts"][0]["size_bytes"] = bytes.len().into();
        let id = store
            .publish_manifest(&serde_json::to_vec(&manifest).expect("json"))
            .expect("publish");
        let mut start = reference('a', id);
        start.exact_state_digest = digest;
        assert_eq!(
            verify_checkpoint(&store, &start, &blob('c'), &blob('d')),
            VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure)
        );
    }
}

#[test]
fn canonical_manifest_bytes_are_required_even_when_identifier_matches() {
    let store = ExactArtifactStore::new(workspace("manifest-canonical"));
    let payload = store.stage_blob(PAYLOAD).expect("stage");
    let mut bytes = manifest_bytes('a', &payload, &payload);
    bytes.push(b'\n');
    let id = store
        .publish_manifest(&bytes)
        .expect("raw store accepts bytes");
    assert_eq!(
        verify_checkpoint(&store, &reference('a', id), &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::MalformedManifest)
    );
}

#[test]
fn reference_boundary_cannot_disagree_with_the_stored_capture() {
    let store = ExactArtifactStore::new(workspace("boundary"));
    let payload = store.stage_blob(PAYLOAD).expect("stage");
    let id = publish(&store, 'a', &payload, &payload);
    let mut start = reference('a', id);
    start.boundary_phase = "SHOP".to_owned();
    assert_eq!(
        verify_checkpoint(&store, &start, &blob('c'), &blob('d')),
        VerificationOutcome::Rejected(VerificationFailure::IntegrityFailure)
    );
}
