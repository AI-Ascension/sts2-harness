// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    ExactArtifactStore, ExactAssurance, ExactCheckpointReference, ExactStateDigest, GateError,
    RestoreEvidence, RestoreGate, RestoreReceipt, SessionError, admit_session,
};

const PAYLOAD: &[u8] =
    include_bytes!("../../../protocol-artifact/exact-state-v1/golden/state.canonical");

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-session-{name}-{}-{nonce}",
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

struct Fixture {
    store: ExactArtifactStore,
    reference: ExactCheckpointReference,
}

fn fixture(name: &str, state_seed: char) -> Fixture {
    let store = ExactArtifactStore::new(workspace(name));
    let payload = store.stage_blob(PAYLOAD).expect("payload stages");
    let restore = store.stage_blob(b"restore").expect("restore stages");
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "canonical_profile": "asc-jcs-state-v1",
        "boundary": {"kind":"decision", "phase":"CONTRACT_FIXTURE"},
        "origin": {"run_id":"synthetic", "generation":0},
        "parent_checkpoint_id": null,
        "exact_state_digest": state(state_seed).as_str(),
        "canonical_payload": {"digest": payload.as_str(), "role":"exact_state_payload", "codec":"asc-jcs-state-v1", "size_bytes":PAYLOAD.len()},
        "restore_artifacts": [{"digest": restore.as_str(), "role":"snapshot", "codec":"test-raw-v1", "size_bytes":7}],
        "compatibility_digest": blob('c'),
        "coverage_contract_digest": blob('d'),
    }))
    .expect("manifest serializes");
    let identifier = store
        .publish_manifest(&manifest)
        .expect("manifest publishes");
    let reference = ExactCheckpointReference {
        exact_state_digest: state(state_seed),
        exact_checkpoint_id: identifier,
        boundary_kind: "decision".to_owned(),
        boundary_phase: "CONTRACT_FIXTURE".to_owned(),
        assurance: ExactAssurance::RestoreSupported,
    };
    Fixture { store, reference }
}

fn receipt(fixture: &Fixture, epoch: u64, evidence: RestoreEvidence) -> RestoreReceipt {
    RestoreReceipt {
        checkpoint_id: fixture.reference.exact_checkpoint_id.clone(),
        source_state_digest: fixture.reference.exact_state_digest.clone(),
        observed_state_digest: fixture.reference.exact_state_digest.clone(),
        compatibility_digest: blob('c'),
        coverage_contract_digest: blob('d'),
        execution_epoch: epoch,
        destination: "runtime:one".to_owned(),
        evidence,
    }
}

fn gate() -> RestoreGate {
    RestoreGate::new(&blob('c'), &blob('d')).expect("gate builds")
}

#[test]
fn admission_requires_both_a_verified_restore_and_verified_artifacts() {
    let fixture = fixture("happy", 'a');
    let mut gate = gate();
    let admission = admit_session(
        &fixture.store,
        &mut gate,
        &fixture.reference,
        &receipt(&fixture, 1, RestoreEvidence::RestoreVerified),
        &blob('c'),
        &blob('d'),
    )
    .expect("session admits");
    assert_eq!(admission.execution_epoch, 1);
    assert_eq!(admission.destination, "runtime:one");
    assert!(admission.evidence.integrity_verified);
    assert!(admission.evidence.restore_verified);
    assert!(!admission.continuation_certified());
    assert_eq!(gate.current_epoch(), 1);
    assert!(gate.admit_decision(1).is_ok());
}

#[test]
fn integrity_only_evidence_never_admits_a_session() {
    let fixture = fixture("integrity-only", 'a');
    let mut gate = gate();
    let refused = admit_session(
        &fixture.store,
        &mut gate,
        &fixture.reference,
        &receipt(&fixture, 1, RestoreEvidence::IntegrityVerified),
        &blob('c'),
        &blob('d'),
    )
    .expect_err("integrity alone is not a restore");
    assert_eq!(refused, SessionError::Gate(GateError::NotRestoreVerified));
    assert_eq!(gate.current_epoch(), 0);
}

#[test]
fn tampered_artifacts_block_admission_despite_a_valid_receipt() {
    let fixture = fixture("tampered", 'a');
    let blob_hex = fixture
        .store
        .stored_blobs()
        .expect("blobs list")
        .first()
        .expect("a blob exists")
        .as_str()
        .trim_start_matches("sha256:")
        .to_owned();
    let path = fixture
        .store
        .root_directory()
        .join("exact")
        .join("blobs")
        .join(&blob_hex[..2])
        .join(&blob_hex);
    fs::write(&path, b"tampered").expect("tamper write");

    let mut gate = gate();
    let refused = admit_session(
        &fixture.store,
        &mut gate,
        &fixture.reference,
        &receipt(&fixture, 1, RestoreEvidence::RestoreVerified),
        &blob('c'),
        &blob('d'),
    )
    .expect_err("tampered artifacts block admission");
    assert!(matches!(refused, SessionError::Verification(_)));
    assert_eq!(gate.current_epoch(), 0);
}

#[test]
fn continuation_certified_receipts_mark_the_session_and_fence_old_epochs() {
    let fixture = fixture("continuation", 'a');
    let mut gate = gate();
    let admission = admit_session(
        &fixture.store,
        &mut gate,
        &fixture.reference,
        &receipt(&fixture, 1, RestoreEvidence::ContinuationCertified),
        &blob('c'),
        &blob('d'),
    )
    .expect("session admits");
    assert!(admission.continuation_certified());

    let second = admit_session(
        &fixture.store,
        &mut gate,
        &fixture.reference,
        &receipt(&fixture, 2, RestoreEvidence::RestoreVerified),
        &blob('c'),
        &blob('d'),
    )
    .expect("second restore admits a new session");
    assert_eq!(second.execution_epoch, 2);
    assert!(!second.continuation_certified());
    assert_eq!(
        gate.admit_decision(1).expect_err("old epoch is fenced"),
        GateError::StaleEpoch
    );
    assert!(gate.admit_decision(2).is_ok());
}

#[test]
fn incompatible_contracts_are_rejected_before_the_gate_opens() {
    let fixture = fixture("incompatible", 'a');
    let mut gate = gate();
    let refused = admit_session(
        &fixture.store,
        &mut gate,
        &fixture.reference,
        &receipt(&fixture, 1, RestoreEvidence::RestoreVerified),
        &blob('e'),
        &blob('d'),
    )
    .expect_err("compatibility mismatch blocks admission");
    assert!(matches!(refused, SessionError::Verification(_)));
    assert_eq!(gate.current_epoch(), 0);
}

#[test]
fn source_and_destination_contracts_must_agree_before_epoch_advance() {
    let fixture = fixture("profile-binding", 'a');
    for (compatibility, coverage) in [(blob('e'), blob('d')), (blob('c'), blob('e'))] {
        let mut destination_gate = RestoreGate::new(&compatibility, &coverage).expect("gate");
        let mut destination = receipt(&fixture, 1, RestoreEvidence::RestoreVerified);
        destination.compatibility_digest = compatibility;
        destination.coverage_contract_digest = coverage;
        assert_eq!(
            admit_session(
                &fixture.store,
                &mut destination_gate,
                &fixture.reference,
                &destination,
                &blob('c'),
                &blob('d')
            ),
            Err(SessionError::Gate(GateError::Incompatible))
        );
        assert_eq!(destination_gate.current_epoch(), 0);
        assert!(destination_gate.admission().is_none());
    }
}
