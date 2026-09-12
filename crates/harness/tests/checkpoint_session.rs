// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{
    ExactArtifactStore, ExactAssurance, ExactCheckpointReference, ExactStateDigest, GateError,
    RestoreEvidence, RestoreGate, RestoreReceipt, SessionError, admit_session,
};

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
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn state(seed: char) -> ExactStateDigest {
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
    let payload = store.stage_blob(b"payload").expect("payload stages");
    let restore = store.stage_blob(b"restore").expect("restore stages");
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "exact_state_digest": state(state_seed).as_str(),
        "canonical_payload": {"digest": payload.as_str()},
        "restore_artifacts": [{"digest": restore.as_str()}],
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
        boundary_phase: "COMBAT".to_owned(),
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
