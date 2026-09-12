// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;

use sts2_harness::{
    CapabilityError, CapabilityReport, CaptureFailure, CheckpointEvidence, CheckpointMode,
    RestoreFailure, UnsupportedPhase,
};

fn evidence() -> CheckpointEvidence {
    CheckpointEvidence {
        captured: true,
        durable: true,
        integrity_verified: true,
        restore_verified: true,
        continuation_certified: true,
        producer: "harness:test".to_owned(),
    }
}

fn report() -> CapabilityReport {
    CapabilityReport {
        adapter_id: "fixture.adapter".to_owned(),
        adapter_version: "0.0.0-fixture".to_owned(),
        canonical_profile: "asc-jcs-state-v1".to_owned(),
        supported_phases: vec!["COMBAT".to_owned(), "REWARD".to_owned()],
        unsupported_phases: vec![UnsupportedPhase {
            phase: "MULTIPLAYER".to_owned(),
            reason: "no coordinated cut capture".to_owned(),
        }],
        capture_mode: CheckpointMode::RestoreVerified,
        coverage_contract_digest: Some(format!("sha256:{}", "a".repeat(64))),
        restore_verified_evidence: true,
    }
}

#[test]
fn evidence_levels_imply_each_lower_level() {
    let complete = evidence();
    complete.validate().expect("complete evidence is coherent");
    assert_eq!(complete.highest_level(), "continuation_certified");

    for mutate in [
        |e: &mut CheckpointEvidence| e.durable = false,
        |e: &mut CheckpointEvidence| e.integrity_verified = false,
        |e: &mut CheckpointEvidence| e.restore_verified = false,
        |e: &mut CheckpointEvidence| e.captured = false,
    ] {
        let mut broken = evidence();
        mutate(&mut broken);
        assert_eq!(
            broken.validate().expect_err("chain broken"),
            CapabilityError::BrokenEvidenceChain
        );
    }

    let mut capture_only = evidence();
    capture_only.durable = false;
    capture_only.integrity_verified = false;
    capture_only.restore_verified = false;
    capture_only.continuation_certified = false;
    assert_eq!(capture_only.highest_level(), "captured");
    let mut nothing = capture_only.clone();
    nothing.captured = false;
    assert_eq!(nothing.highest_level(), "none");
}

#[test]
fn capabilities_refuse_unsupported_claims() {
    let valid = report();
    valid.validate().expect("report is coherent");
    assert!(valid.supports("COMBAT"));
    assert!(!valid.supports("MULTIPLAYER"));
    assert_eq!(
        valid.unsupported_reason("MULTIPLAYER"),
        Some("no coordinated cut capture")
    );

    let mut missing_coverage = report();
    missing_coverage.coverage_contract_digest = None;
    assert_eq!(
        missing_coverage.validate().expect_err("coverage required"),
        CapabilityError::MissingCoverage
    );

    let mut unbacked_restore = report();
    unbacked_restore.restore_verified_evidence = false;
    assert_eq!(
        unbacked_restore
            .validate()
            .expect_err("restore evidence required"),
        CapabilityError::UnsupportedRestoreClaim
    );

    let mut inconsistent = report();
    inconsistent.capture_mode = CheckpointMode::CaptureOnly;
    assert_eq!(
        inconsistent
            .validate()
            .expect_err("mode contradicts evidence"),
        CapabilityError::InconsistentMode
    );

    let mut observation_only = report();
    observation_only.capture_mode = CheckpointMode::ObservationOnly;
    observation_only.coverage_contract_digest = None;
    observation_only.restore_verified_evidence = false;
    observation_only
        .validate()
        .expect("observation-only is coherent");
    assert!(!observation_only.capture_mode.allows_exact_claims());
}

#[test]
fn failure_taxonomies_stay_distinct_and_labeled() {
    let capture = [
        CaptureFailure::UnsafeBoundary,
        CaptureFailure::UnsupportedProfile,
        CaptureFailure::IncompleteCoverage,
        CaptureFailure::InconsistentCapture,
        CaptureFailure::AuthorityLost,
        CaptureFailure::Timeout,
        CaptureFailure::Cancelled,
        CaptureFailure::StorageFailure,
        CaptureFailure::ProducerFailure,
    ];
    let restore = [
        RestoreFailure::ManifestMissing,
        RestoreFailure::IntegrityFailure,
        RestoreFailure::IncompatibleProfile,
        RestoreFailure::CoverageIncomplete,
        RestoreFailure::AuthorityLost,
        RestoreFailure::DestinationBusy,
        RestoreFailure::RecaptureMismatch,
        RestoreFailure::StorageFailure,
    ];
    let capture_labels: BTreeSet<&str> = capture.iter().map(|failure| failure.as_str()).collect();
    let restore_labels: BTreeSet<&str> = restore.iter().map(|failure| failure.as_str()).collect();
    assert_eq!(capture_labels.len(), capture.len());
    assert_eq!(restore_labels.len(), restore.len());
    let shared: BTreeSet<&str> = capture_labels
        .intersection(&restore_labels)
        .copied()
        .collect();
    assert_eq!(
        shared,
        BTreeSet::from(["authority_lost", "storage_failure"])
    );
    assert!(capture_labels.contains("unsafe_boundary"));
    assert!(restore_labels.contains("recapture_mismatch"));
}

#[test]
fn public_evidence_summary_carries_no_digests() {
    let summary = evidence().public_summary().expect("summary builds");
    let json = serde_json::to_string(&summary).expect("summary serializes");
    assert!(json.contains("\"level\":\"continuation_certified\""));
    assert!(json.contains("\"restore_verified\":true"));
    for forbidden in ["sha256", "asc-state", "asc-checkpoint", "producer"] {
        assert!(!json.contains(forbidden), "summary leaked {forbidden}");
    }
    let mut uncertified = evidence();
    uncertified.continuation_certified = false;
    assert_eq!(
        uncertified.public_summary().expect("summary").level,
        "restore_verified"
    );
}
