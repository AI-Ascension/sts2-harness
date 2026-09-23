// SPDX-License-Identifier: MIT

#![allow(clippy::panic, clippy::unwrap_used, dead_code)]

// AC3: same-case model trials verify the same initial native witness; later action/RNG divergence
// is not labelled a determinism defect.

#[path = "benchmark_suite/fixtures.rs"]
mod fixtures;

use fixtures::{key_of, manifest, witness};
use sts2_harness::ExactCheckpointError;
use sts2_harness::benchmark_manifest::suite::{
    NativeWitness, ReportError, TrialOutcome, TrialResult, audit_initial_witness,
};

fn started(key: &str, result: TrialResult, marker: u8) -> TrialOutcome {
    TrialOutcome::completed(key, result).with_witness(witness(marker))
}

#[test]
fn divergent_outcomes_share_one_verified_start_group() {
    let outcomes = vec![
        started(
            &key_of("case-alpha", "policy-exo", 0),
            TrialResult::Victory,
            0xaa,
        ),
        started(
            &key_of("case-alpha", "policy-exo", 1),
            TrialResult::Defeat,
            0xaa,
        ),
        started(
            &key_of("case-alpha", "policy-local", 0),
            TrialResult::Victory,
            0xaa,
        ),
        started(
            &key_of("case-alpha", "policy-local", 1),
            TrialResult::Victory,
            0xaa,
        ),
    ];
    let audit = audit_initial_witness(&manifest(), &outcomes).unwrap();
    assert_eq!(audit.groups.len(), 1);
    let group = &audit.groups[0];
    assert_eq!(group.case_id, "case-alpha");
    assert_eq!(group.witness, witness(0xaa).as_str());
    assert_eq!(
        group.members.len(),
        4,
        "all four verified trials share one start"
    );
    assert_eq!(
        audit.unverified.len(),
        4,
        "case-beta never started natively"
    );
    assert!(
        audit
            .unverified
            .iter()
            .all(|member| member.case_id == "case-beta")
    );
}

#[test]
fn conflicting_initial_witnesses_are_a_defect() {
    let outcomes = vec![
        started(
            &key_of("case-alpha", "policy-exo", 0),
            TrialResult::Victory,
            0xaa,
        ),
        started(
            &key_of("case-alpha", "policy-exo", 1),
            TrialResult::Victory,
            0xbb,
        ),
    ];
    assert_eq!(
        audit_initial_witness(&manifest(), &outcomes).unwrap_err(),
        ReportError::WitnessMismatch("case-alpha".to_owned())
    );
}

#[test]
fn unverified_trials_are_excluded_from_exact_start_groups() {
    let outcomes = vec![
        started(
            &key_of("case-alpha", "policy-exo", 0),
            TrialResult::Victory,
            0xaa,
        ),
        TrialOutcome::completed(&key_of("case-alpha", "policy-exo", 1), TrialResult::Defeat),
    ];
    let audit = audit_initial_witness(&manifest(), &outcomes).unwrap();
    assert_eq!(audit.groups.len(), 1);
    assert_eq!(audit.groups[0].members.len(), 1);
    assert_eq!(
        audit.unverified.len(),
        7,
        "an unverified start is never blended in"
    );
}

#[test]
fn witness_construction_requires_the_exact_state_namespace() {
    assert!(matches!(
        NativeWitness::parse(
            "asc-checkpoint:v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
        ),
        Err(ExactCheckpointError::InvalidDigest)
    ));
    assert!(matches!(
        NativeWitness::parse("asc-state:v1:sha256:ABCDEF"),
        Err(ExactCheckpointError::InvalidDigest)
    ));
    assert!(NativeWitness::parse(witness(0x01).as_str()).is_ok());
}
