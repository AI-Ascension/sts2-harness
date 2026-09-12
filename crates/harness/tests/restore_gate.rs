// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    ExactAssurance, ExactCheckpointId, ExactCheckpointReference, ExactStateDigest, GateError,
    RestoreEvidence, RestoreGate, RestoreReceipt,
};

fn digest(seed: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("state digest is valid")
}

fn checkpoint(seed: char) -> ExactCheckpointId {
    ExactCheckpointId::parse(&format!(
        "asc-checkpoint:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("checkpoint is valid")
}

fn blob(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn reference() -> ExactCheckpointReference {
    ExactCheckpointReference {
        exact_state_digest: digest('a'),
        exact_checkpoint_id: checkpoint('b'),
        boundary_kind: "decision".to_owned(),
        boundary_phase: "COMBAT".to_owned(),
        assurance: ExactAssurance::RestoreSupported,
    }
}

fn receipt(epoch: u64) -> RestoreReceipt {
    RestoreReceipt {
        checkpoint_id: checkpoint('b'),
        source_state_digest: digest('a'),
        observed_state_digest: digest('a'),
        compatibility_digest: blob('c'),
        coverage_contract_digest: blob('d'),
        execution_epoch: epoch,
        destination: "runtime:one".to_owned(),
        evidence: RestoreEvidence::RestoreVerified,
    }
}

fn gate() -> RestoreGate {
    RestoreGate::new(&blob('c'), &blob('d')).expect("gate builds")
}

#[test]
fn decisions_require_a_verified_restore_and_a_current_epoch() {
    let mut gate = gate();
    assert_eq!(
        gate.admit_decision(1).expect_err("no restore yet"),
        GateError::NotAdmitted
    );
    let admission = gate
        .admit(&reference(), &receipt(1))
        .expect("restore admitted");
    assert_eq!(admission.execution_epoch, 1);
    assert_eq!(gate.current_epoch(), 1);
    assert!(gate.admit_decision(1).is_ok());
    assert_eq!(
        gate.admit_decision(0).expect_err("old epoch"),
        GateError::StaleEpoch
    );
}

#[test]
fn integrity_evidence_without_restore_is_rejected() {
    let mut gate = gate();
    let mut unverified = receipt(1);
    unverified.evidence = RestoreEvidence::IntegrityVerified;
    assert_eq!(
        gate.admit(&reference(), &unverified)
            .expect_err("not verified"),
        GateError::NotRestoreVerified
    );
    assert_eq!(gate.current_epoch(), 0);
}

#[test]
fn digest_mismatch_is_reported_separately_from_compatibility() {
    let mut gate = gate();
    let mut recaptured_other = receipt(1);
    recaptured_other.observed_state_digest = digest('e');
    assert_eq!(
        gate.admit(&reference(), &recaptured_other)
            .expect_err("recapture differs"),
        GateError::DigestMismatch
    );
    let mut other_checkpoint = receipt(1);
    other_checkpoint.checkpoint_id = checkpoint('f');
    assert_eq!(
        gate.admit(&reference(), &other_checkpoint)
            .expect_err("wrong checkpoint"),
        GateError::CheckpointMismatch
    );
    let mut incompatible = receipt(1);
    incompatible.compatibility_digest = blob('f');
    assert_eq!(
        gate.admit(&reference(), &incompatible)
            .expect_err("wrong profile"),
        GateError::Incompatible
    );
}

#[test]
fn replayed_and_regressing_epochs_are_rejected() {
    let mut gate = gate();
    gate.admit(&reference(), &receipt(2))
        .expect("first restore");
    assert_eq!(
        gate.admit(&reference(), &receipt(2))
            .expect_err("replayed epoch"),
        GateError::StaleEpoch
    );
    assert_eq!(
        gate.admit(&reference(), &receipt(1))
            .expect_err("regressing epoch"),
        GateError::StaleEpoch
    );
    gate.admit(&reference(), &receipt(3))
        .expect("second restore");
    assert_eq!(gate.current_epoch(), 3);
    assert_eq!(
        gate.admit_decision(2).expect_err("pre-restore epoch"),
        GateError::StaleEpoch
    );
    assert!(gate.admit_decision(3).is_ok());
}

#[test]
fn invalid_destinations_and_digests_are_rejected() {
    let mut gate = gate();
    let mut blank = receipt(1);
    blank.destination = String::new();
    assert_eq!(
        gate.admit(&reference(), &blank)
            .expect_err("blank destination"),
        GateError::InvalidDestination
    );
    assert_eq!(
        RestoreGate::new("not-a-digest", &blob('d')).expect_err("bad compatibility digest"),
        GateError::InvalidDigest
    );
    let mut continuation = receipt(1);
    continuation.evidence = RestoreEvidence::ContinuationCertified;
    assert!(gate.admit(&reference(), &continuation).is_ok());
    assert_eq!(
        RestoreEvidence::IntegrityVerified.as_str(),
        "integrity_verified"
    );
    assert!(!RestoreEvidence::IntegrityVerified.is_restore_verified());
}
