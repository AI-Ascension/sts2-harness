// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    ExactStateDigest, TraceOutcome, TransitionRecord, TransitionTrace, diagnose_traces,
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

fn trace(states: &[char], action: &str) -> TransitionTrace {
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
            external_input_digest: None,
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
fn state_divergence_reports_privileged_identities_and_a_safe_public_status() {
    let expected = trace(&['a', 'b', 'c', 'd'], "play_card");
    let actual = trace(&['a', 'b', '1', 'd'], "play_card");
    let diagnosis = diagnose_traces(&expected, &actual).expect("diagnosis");
    assert_eq!(diagnosis.outcome, TraceOutcome::StateDivergence);
    assert_eq!(diagnosis.last_equal_ordinal, Some(1));
    assert_eq!(diagnosis.first_unequal_ordinal, Some(2));
    assert_eq!(diagnosis.expected_state_digest, Some(digest('c')));
    assert_eq!(diagnosis.actual_state_digest, Some(digest('1')));
    assert!(diagnosis.has_privileged_identities());

    let public =
        serde_json::to_string(&diagnosis.public_status()).expect("public status serializes");
    assert!(public.contains("\"outcome\":\"state_divergence\""));
    assert!(public.contains("\"first_unequal_ordinal\":2"));
    assert!(!public.contains("asc-state"));
    assert!(!public.contains("sha256"));
    assert!(!public.contains("aaaaaaaa"));
}

#[test]
fn identical_traces_carry_no_privileged_identities() {
    let expected = trace(&['a', 'b', 'c'], "play_card");
    let diagnosis = diagnose_traces(&expected, &expected).expect("diagnosis");
    assert_eq!(diagnosis.outcome, TraceOutcome::IdenticalOverRecordedRange);
    assert!(!diagnosis.has_privileged_identities());
    assert_eq!(diagnosis.expected_state_digest, None);
    assert_eq!(
        diagnosis.public_status().outcome,
        "identical_over_recorded_range"
    );
}

#[test]
fn missing_capture_reports_the_unobserved_boundary() {
    let expected = trace(&['a', 'b', 'c', 'd'], "play_card");
    let short = trace(&['a', 'b', 'c'], "play_card");
    let diagnosis = diagnose_traces(&expected, &short).expect("diagnosis");
    assert_eq!(diagnosis.outcome, TraceOutcome::MissingCapture);
    assert_eq!(diagnosis.first_unequal_ordinal, Some(3));
    assert_eq!(diagnosis.unobserved_records, 1);
    assert_eq!(diagnosis.expected_state_digest, Some(digest('d')));
    assert_eq!(diagnosis.actual_state_digest, None);
}

#[test]
fn incompatible_profiles_are_classified_without_privileged_identities() {
    let expected = trace(&['a', 'b', 'c'], "play_card");
    let mut other = trace(&['a', 'b', 'c'], "play_card");
    other.profile = "other-v1".to_owned();
    let diagnosis = diagnose_traces(&expected, &other).expect("diagnosis");
    assert_eq!(diagnosis.outcome, TraceOutcome::IncompatibleProfile);
    assert!(!diagnosis.has_privileged_identities());
    assert_eq!(diagnosis.public_status().outcome, "incompatible_profile");
}

#[test]
fn outcome_labels_are_stable() {
    assert_eq!(TraceOutcome::DifferentAction.as_str(), "different_action");
    assert_eq!(
        TraceOutcome::DifferentExternalInput.as_str(),
        "different_external_input"
    );
    assert_eq!(TraceOutcome::RestoreMismatch.as_str(), "restore_mismatch");
    assert_eq!(TraceOutcome::UnalignedTrace.as_str(), "unaligned_trace");
    assert_eq!(
        TraceOutcome::InsufficientCoverage.as_str(),
        "insufficient_coverage"
    );
}
