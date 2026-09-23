// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use sts2_harness::{
    AdmissionRefusal, DivergenceLimits, ExactStateDigest, ReproducerError, TraceBundleManifest,
    TraceDivergenceError, TraceOutcome, TransitionRecord, TransitionTrace, diagnose_bundles,
    export_reproducer_prefix,
};

fn digest(seed: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("digest is valid")
}

fn blob(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn ordinals(count: usize) -> Vec<u64> {
    (1..=u64::try_from(count).expect("count fits")).collect()
}

fn build(
    profile: &str,
    states: &[char],
    ordinals: &[u64],
    action: &str,
    schema: &str,
) -> TransitionTrace {
    let mut records = Vec::new();
    let mut previous: Option<String> = None;
    for (index, ordinal) in ordinals.iter().enumerate() {
        let record = TransitionRecord {
            ordinal: *ordinal,
            boundary_kind: "decision".to_owned(),
            boundary_phase: "COMBAT".to_owned(),
            before: digest(states[index]),
            after: digest(states[index + 1]),
            action_key: action.to_owned(),
            action_schema: schema.to_owned(),
            catalog_witness: Some(blob('c')),
            external_input_digest: None,
            previous_commitment: previous.clone(),
        };
        previous = Some(record.commitment().expect("commitment computes"));
        records.push(record);
    }
    TransitionTrace {
        profile: profile.to_owned(),
        source_state: digest(states[0]),
        records,
    }
}

fn simple(states: &[char]) -> TransitionTrace {
    let count = states.len() - 1;
    build(
        "fixture-v1",
        states,
        &ordinals(count),
        "play_card",
        "fixture.action.v1",
    )
}

fn manifest(trace: &TransitionTrace) -> TraceBundleManifest {
    TraceBundleManifest::for_trace(trace, "bundle-ref").expect("manifest derives")
}

fn refused(
    result: Result<sts2_harness::AdmittedDiagnosis, TraceDivergenceError>,
) -> AdmissionRefusal {
    match result {
        Err(TraceDivergenceError::Refused(reason)) => reason,
        other => panic!("expected an admission refusal, got {other:?}"),
    }
}

#[test]
fn reconverging_endpoints_still_report_the_first_divergence() {
    let expected = simple(&['a', 'b', 'c', 'd', 'e']);
    let actual = simple(&['a', 'b', '1', 'd', 'e']);
    let report = diagnose_bundles(
        &manifest(&expected),
        &expected,
        &manifest(&actual),
        &actual,
        &DivergenceLimits::default(),
    )
    .expect("admitted");
    assert_eq!(report.diagnosis.outcome, TraceOutcome::StateDivergence);
    assert_eq!(report.diagnosis.first_unequal_ordinal, Some(2));
    assert_eq!(report.diagnosis.last_equal_ordinal, Some(1));
    assert_eq!(report.diagnosis.expected_state_digest, Some(digest('c')));
    assert_eq!(report.diagnosis.actual_state_digest, Some(digest('1')));
    assert!(!report.privileged_differences.is_empty());

    let public = serde_json::to_string(&report.public_status()).expect("public serializes");
    assert!(public.contains("\"outcome\":\"state_divergence\""));
    assert!(public.contains("\"first_unequal_ordinal\":2"));
    assert!(!public.contains("asc-state"));
    assert!(!public.contains("sha256"));
    assert!(!public.contains("cccc"));
}

#[test]
fn unequal_lengths_report_missing_capture_without_a_field_diff() {
    let expected = simple(&['a', 'b', 'c', 'd']);
    let actual = simple(&['a', 'b', 'c']);
    let report = diagnose_bundles(
        &manifest(&expected),
        &expected,
        &manifest(&actual),
        &actual,
        &DivergenceLimits::default(),
    )
    .expect("admitted");
    assert_eq!(report.diagnosis.outcome, TraceOutcome::MissingCapture);
    assert_eq!(report.diagnosis.first_unequal_ordinal, Some(3));
    assert_eq!(report.diagnosis.unobserved_records, 1);
    assert!(report.privileged_differences.is_empty());
    assert_eq!(report.reproducer.expect("prefix").boundary_ordinal, 3);
}

#[test]
fn non_sequential_ordinals_are_not_guessed_into_alignment() {
    let expected = build(
        "fixture-v1",
        &['a', 'b', 'c', 'd'],
        &[1, 2, 3],
        "play_card",
        "fixture.action.v1",
    );
    let actual = build(
        "fixture-v1",
        &['a', 'b', 'c', 'd'],
        &[1, 3, 4],
        "play_card",
        "fixture.action.v1",
    );
    let report = diagnose_bundles(
        &manifest(&expected),
        &expected,
        &manifest(&actual),
        &actual,
        &DivergenceLimits::default(),
    )
    .expect("admitted");
    assert_eq!(report.diagnosis.outcome, TraceOutcome::UnalignedTrace);
    assert_eq!(report.diagnosis.first_unequal_ordinal, Some(2));
    assert!(report.privileged_differences.is_empty());
}

#[test]
fn a_duplicate_ordinal_chain_is_refused_before_comparison() {
    let expected = simple(&['a', 'b', 'c']);
    let duplicate = build(
        "fixture-v1",
        &['a', 'b', 'c'],
        &[1, 1],
        "play_card",
        "fixture.action.v1",
    );
    let mut claimed = manifest(&expected);
    claimed.record_count = 99;
    let reason = refused(diagnose_bundles(
        &claimed,
        &expected,
        &manifest(&expected),
        &expected,
        &DivergenceLimits::default(),
    ));
    assert_eq!(reason, AdmissionRefusal::ClosureMismatch);

    let bound = TraceBundleManifest {
        bundle_ref: "bundle-ref".to_owned(),
        profile: duplicate.profile.clone(),
        action_schema_revisions: vec!["fixture.action.v1".to_owned()],
        source_state: duplicate.source_state.clone(),
        record_count: 2,
        closure_commitment: None,
    };
    let reason = refused(diagnose_bundles(
        &bound,
        &duplicate,
        &manifest(&expected),
        &expected,
        &DivergenceLimits::default(),
    ));
    assert_eq!(reason, AdmissionRefusal::ClosureMismatch);
}

#[test]
fn incompatible_schema_revisions_and_profiles_are_refused() {
    let expected = build(
        "fixture-v1",
        &['a', 'b', 'c'],
        &[1, 2],
        "play_card",
        "fixture.action.v1",
    );
    let newer = build(
        "fixture-v1",
        &['a', 'b', 'c'],
        &[1, 2],
        "play_card",
        "fixture.action.v2",
    );
    assert_eq!(
        refused(diagnose_bundles(
            &manifest(&expected),
            &expected,
            &manifest(&newer),
            &newer,
            &DivergenceLimits::default(),
        )),
        AdmissionRefusal::IncompatibleSchema
    );

    let other_profile = build(
        "other-v1",
        &['a', 'b', 'c'],
        &[1, 2],
        "play_card",
        "fixture.action.v1",
    );
    assert_eq!(
        refused(diagnose_bundles(
            &manifest(&expected),
            &expected,
            &manifest(&other_profile),
            &other_profile,
            &DivergenceLimits::default(),
        )),
        AdmissionRefusal::IncompatibleProfile
    );
}

#[test]
fn zero_bounds_and_empty_coverage_fail_closed() {
    let expected = simple(&['a', 'b', 'c']);
    let actual = simple(&['a', 'b', 'c']);
    assert_eq!(
        refused(diagnose_bundles(
            &manifest(&expected),
            &expected,
            &manifest(&actual),
            &actual,
            &DivergenceLimits::new(0, 8, 8),
        )),
        AdmissionRefusal::InvalidLimits
    );

    let empty = TransitionTrace {
        profile: "fixture-v1".to_owned(),
        source_state: digest('a'),
        records: Vec::new(),
    };
    assert_eq!(
        refused(diagnose_bundles(
            &manifest(&empty),
            &empty,
            &manifest(&actual),
            &actual,
            &DivergenceLimits::default(),
        )),
        AdmissionRefusal::EmptyCoverage
    );
}

#[test]
fn the_reproducer_is_a_verified_prefix_and_leaves_the_source_unchanged() {
    let expected = simple(&['a', 'b', 'c', 'd', 'e']);
    let actual = simple(&['a', 'b', '1', 'd', 'e']);
    let original = expected.clone();
    let report = diagnose_bundles(
        &manifest(&expected),
        &expected,
        &manifest(&actual),
        &actual,
        &DivergenceLimits::default(),
    )
    .expect("admitted");
    let prefix = report.reproducer.expect("prefix");
    assert_eq!(prefix.boundary_ordinal, 2);
    assert_eq!(prefix.records.len(), 2);
    assert_eq!(
        prefix.boundary_commitment,
        prefix.records[1].commitment().expect("commitment")
    );
    prefix
        .validate_against_source(&expected)
        .expect("prefix is a verified head");
    assert_eq!(expected, original, "the source trace is never mutated");
}

#[test]
fn an_unrecorded_or_oversized_boundary_is_refused() {
    let expected = simple(&['a', 'b', 'c', 'd']);
    assert_eq!(
        export_reproducer_prefix(&expected, 99, &DivergenceLimits::default()),
        Err(ReproducerError::BoundaryNotRecorded)
    );
    assert_eq!(
        export_reproducer_prefix(&expected, 3, &DivergenceLimits::new(10, 10, 4)),
        Err(ReproducerError::PrefixTooLarge)
    );
    assert_eq!(
        export_reproducer_prefix(&expected, 3, &DivergenceLimits::new(1, 10, 4096)),
        Err(ReproducerError::PrefixTooLarge)
    );
}

#[test]
fn large_traces_report_record_truncation() {
    let expected = simple(&['a', 'b', 'c', 'd', 'e', 'f']);
    let actual = simple(&['a', 'b', 'c', 'd', 'e', 'f']);
    let report = diagnose_bundles(
        &manifest(&expected),
        &expected,
        &manifest(&actual),
        &actual,
        &DivergenceLimits::new(2, 64, 8192),
    )
    .expect("admitted");
    assert!(report.truncation.records_truncated);
    assert!(report.truncation.records_examined <= 2);
    assert_eq!(
        report.diagnosis.outcome,
        TraceOutcome::IdenticalOverRecordedRange
    );
}

#[test]
fn privileged_difference_entries_are_bounded_and_reported() {
    let expected = build(
        "fixture-v1",
        &['a', 'b', 'c'],
        &[1, 2],
        "play_card",
        "fixture.action.v1",
    );
    let mut actual = build(
        "fixture-v1",
        &['a', 'b', '9'],
        &[1, 2],
        "play_card",
        "fixture.action.v1",
    );
    // Diverge only at the second boundary, changing both the action and the resulting state.
    actual.records[1].action_key = "play_other".to_owned();
    let report = diagnose_bundles(
        &manifest(&expected),
        &expected,
        &manifest(&actual),
        &actual,
        &DivergenceLimits::new(64, 1, 8192),
    )
    .expect("admitted");
    assert_eq!(report.diagnosis.outcome, TraceOutcome::DifferentAction);
    assert_eq!(report.truncation.entries_kept, 1);
    assert_eq!(report.truncation.entries_dropped, 1);
    assert!(report.truncation.bytes_excluded);
    assert_eq!(report.privileged_differences[0].field, "after");
}
