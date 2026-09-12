// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    ExactStateDigest, TraceOutcome, TransitionError, TransitionRecord, TransitionTrace,
    compare_traces,
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

fn trace(states: &[char], action: &str, input: Option<String>) -> TransitionTrace {
    let mut records = Vec::new();
    let mut previous: Option<String> = None;
    for (index, pair) in states.windows(2).enumerate() {
        let record = TransitionRecord {
            ordinal: u64::try_from(index).expect("index fits") + 1,
            boundary_kind: "decision".to_owned(),
            boundary_phase: "COMBAT".to_owned(),
            before: digest(pair[0]),
            after: digest(pair[1]),
            action_key: action.to_owned(),
            action_schema: "fixture.action.v1".to_owned(),
            catalog_witness: Some(blob('c')),
            external_input_digest: input.clone(),
            previous_commitment: previous.clone(),
        };
        previous = Some(record.commitment().expect("commitment computes"));
        records.push(record);
    }
    TransitionTrace {
        profile: "fixture-v1".to_owned(),
        source_state: digest(states[0]),
        records,
    }
}

#[test]
fn identical_traces_match_over_the_recorded_range() {
    let left = trace(&['a', 'b', 'c', 'd'], "play_card", Some(blob('e')));
    let right = trace(&['a', 'b', 'c', 'd'], "play_card", Some(blob('e')));
    let comparison = compare_traces(&left, &right).expect("comparison runs");
    assert_eq!(comparison.outcome, TraceOutcome::IdenticalOverRecordedRange);
    assert_eq!(comparison.last_equal_ordinal, Some(3));
    assert_eq!(comparison.first_unequal_ordinal, None);
    assert_eq!(comparison.compared_records, 3);
    assert_eq!(comparison.unobserved_records, 0);
    left.validate().expect("trace is coherent");
}

#[test]
fn action_and_input_differences_are_classified_before_state() {
    let expected = trace(&['a', 'b', 'c'], "play_card", Some(blob('e')));
    let other_action = trace(&['a', 'b', 'c'], "end_turn", Some(blob('e')));
    assert_eq!(
        compare_traces(&expected, &other_action)
            .expect("runs")
            .outcome,
        TraceOutcome::DifferentAction
    );
    let other_input = trace(&['a', 'b', 'c'], "play_card", Some(blob('f')));
    let comparison = compare_traces(&expected, &other_input).expect("runs");
    assert_eq!(comparison.outcome, TraceOutcome::DifferentExternalInput);
    assert_eq!(comparison.last_equal_ordinal, None);
    assert_eq!(comparison.first_unequal_ordinal, Some(1));
}

#[test]
fn reconvergence_still_reports_the_earliest_divergence() {
    let expected = trace(&['a', 'b', 'c', 'd'], "play_card", Some(blob('e')));
    let actual = trace(&['a', 'b', '1', 'd'], "play_card", Some(blob('e')));
    assert_eq!(
        expected.records.last().expect("record").after,
        actual.records.last().expect("record").after,
        "the endpoints match, so endpoint equality must not be used as proof"
    );
    let comparison = compare_traces(&expected, &actual).expect("runs");
    assert_eq!(comparison.outcome, TraceOutcome::StateDivergence);
    assert_eq!(comparison.last_equal_ordinal, Some(1));
    assert_eq!(comparison.first_unequal_ordinal, Some(2));
}

#[test]
fn shorter_and_longer_actual_traces_report_the_unobserved_interval() {
    let expected = trace(&['a', 'b', 'c', 'd'], "play_card", None);
    let short = trace(&['a', 'b', 'c'], "play_card", None);
    let comparison = compare_traces(&expected, &short).expect("runs");
    assert_eq!(comparison.outcome, TraceOutcome::MissingCapture);
    assert_eq!(comparison.last_equal_ordinal, Some(2));
    assert_eq!(comparison.first_unequal_ordinal, Some(3));
    assert_eq!(comparison.unobserved_records, 1);

    let long = trace(&['a', 'b', 'c', 'd', 'e'], "play_card", None);
    let comparison = compare_traces(&expected, &long).expect("runs");
    assert_eq!(comparison.outcome, TraceOutcome::IdenticalOverRecordedRange);
    assert_eq!(comparison.unobserved_records, 1);
}

#[test]
fn genesis_and_profile_mismatches_are_separated() {
    let expected = trace(&['a', 'b', 'c'], "play_card", None);
    let other_origin = trace(&['2', 'b', 'c'], "play_card", None);
    assert_eq!(
        compare_traces(&expected, &other_origin)
            .expect("runs")
            .outcome,
        TraceOutcome::RestoreMismatch
    );
    let mut other_profile = trace(&['a', 'b', 'c'], "play_card", None);
    other_profile.profile = "other-v1".to_owned();
    assert_eq!(
        compare_traces(&expected, &other_profile)
            .expect("runs")
            .outcome,
        TraceOutcome::IncompatibleProfile
    );
}

#[test]
fn unaligned_ordinals_are_reported_as_alignment_failure() {
    let expected = trace(&['a', 'b', 'c'], "play_card", None);
    let mut actual = trace(&['a', 'b', 'c'], "play_card", None);
    actual.records[1].ordinal = 7;
    let comparison = compare_traces(&expected, &actual).expect("runs");
    assert_eq!(comparison.outcome, TraceOutcome::UnalignedTrace);
    assert_eq!(comparison.last_equal_ordinal, Some(1));
    assert_eq!(comparison.first_unequal_ordinal, Some(2));
}

#[test]
fn broken_chains_and_non_monotonic_ordinals_are_rejected() {
    let mut broken = trace(&['a', 'b', 'c'], "play_card", None);
    broken.records[1].previous_commitment =
        Some(format!("asc-transition:v1:sha256:{}", "0".repeat(64)));
    assert_eq!(
        broken.validate(),
        Err(TransitionError::BrokenCommitmentChain)
    );

    let mut descending = trace(&['a', 'b', 'c'], "play_card", None);
    descending.records[1].ordinal = descending.records[0].ordinal;
    assert_eq!(
        descending.validate(),
        Err(TransitionError::NonMonotonicOrdinal)
    );

    let empty = TransitionTrace {
        profile: "fixture-v1".to_owned(),
        source_state: digest('a'),
        records: Vec::new(),
    };
    assert_eq!(
        compare_traces(&empty, &empty).expect("runs").outcome,
        TraceOutcome::InsufficientCoverage
    );
}

#[test]
fn missing_catalog_witness_is_reported_as_missing_capture() {
    let expected = trace(&['a', 'b', 'c'], "play_card", None);
    let mut actual = trace(&['a', 'b', 'c'], "play_card", None);
    actual.records[0].catalog_witness = None;
    let regenerated = actual.records[0].commitment().expect("commitment");
    actual.records[1].previous_commitment = Some(regenerated);
    assert_eq!(
        compare_traces(&expected, &actual).expect("runs").outcome,
        TraceOutcome::MissingCapture
    );
}
