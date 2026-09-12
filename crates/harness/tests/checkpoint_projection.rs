// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    ExactAssurance, ExactCheckpointId, ExactStateDigest, HANDLE_PREFIX, MIN_HANDLE_KEY_BYTES,
    OccurrenceId, ProjectionError, ProjectionKey,
};

fn checkpoint(seed: char) -> ExactCheckpointId {
    ExactCheckpointId::parse(&format!(
        "asc-checkpoint:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("checkpoint id is valid")
}

fn state(seed: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("state digest is valid")
}

fn occurrence(value: &str) -> OccurrenceId {
    OccurrenceId::parse(value).expect("occurrence is valid")
}

fn key(seed: u8) -> ProjectionKey {
    ProjectionKey::new(&[seed; MIN_HANDLE_KEY_BYTES]).expect("key is strong enough")
}

#[test]
fn public_summary_carries_no_privileged_values() {
    let projection = key(1);
    let summary = projection
        .project(
            &checkpoint('b'),
            &state('a'),
            &occurrence("run:one"),
            "decision",
            "COMBAT",
            ExactAssurance::RestoreVerified,
        )
        .expect("summary builds");
    let json = serde_json::to_string(&summary).expect("summary serializes");
    assert!(summary.handle.starts_with(HANDLE_PREFIX));
    assert!(json.contains("\"restore_verified\":true"));
    assert!(json.contains("\"boundary_phase\":\"COMBAT\""));
    assert!(json.contains("\"schema\":\"ascension.exact_checkpoint_reference.v1\""));
    assert!(json.contains("\"reference_version\":\"exact-checkpoint-reference-v1\""));
    assert!(json.contains("\"assurance\":\"restore_verified\""));
    for secret in [
        "a".repeat(64),
        "b".repeat(64),
        "asc-state:v1:sha256:".to_owned(),
        "asc-checkpoint:v1:sha256:".to_owned(),
        "sha256:".to_owned(),
    ] {
        assert!(
            !json.contains(&secret),
            "public summary leaked privileged value {secret}"
        );
    }
}

#[test]
fn handles_are_deterministic_and_scoped() {
    let projection = key(2);
    let first = projection
        .handle(&checkpoint('b'), &state('a'), &occurrence("run:one"))
        .expect("handle");
    let repeat = projection
        .handle(&checkpoint('b'), &state('a'), &occurrence("run:one"))
        .expect("handle");
    let other_occurrence = projection
        .handle(&checkpoint('b'), &state('a'), &occurrence("run:two"))
        .expect("handle");
    let other_state = projection
        .handle(&checkpoint('b'), &state('c'), &occurrence("run:one"))
        .expect("handle");
    assert_eq!(first, repeat);
    assert_ne!(first, other_occurrence);
    assert_ne!(first, other_state);
    assert_eq!(first.len(), HANDLE_PREFIX.len() + 64);
}

#[test]
fn a_different_key_cannot_test_candidate_states() {
    let trusted = key(3);
    let attacker = key(4);
    let handle = trusted
        .handle(&checkpoint('b'), &state('a'), &occurrence("run:one"))
        .expect("handle");
    assert!(
        trusted
            .matches(
                &handle,
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .expect("match runs")
    );
    assert!(
        !attacker
            .matches(
                &handle,
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .expect("match runs")
    );
    assert!(
        !trusted
            .matches(
                &handle,
                &checkpoint('b'),
                &state('c'),
                &occurrence("run:one")
            )
            .expect("candidate comparison runs")
    );
}

#[test]
fn weak_keys_and_invalid_inputs_are_rejected() {
    assert_eq!(
        ProjectionKey::new(&[0_u8; MIN_HANDLE_KEY_BYTES - 1]).expect_err("weak key"),
        ProjectionError::WeakKey
    );
    let projection = key(5);
    assert_eq!(
        projection
            .project(
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one"),
                "",
                "COMBAT",
                ExactAssurance::CaptureOnly
            )
            .expect_err("empty boundary"),
        ProjectionError::InvalidInput
    );
    assert_eq!(
        projection
            .matches(
                "ckpt-h1:short",
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .expect_err("malformed handle"),
        ProjectionError::InvalidHandle
    );
    assert_eq!(
        projection
            .matches(
                &format!("{HANDLE_PREFIX}{}", "A".repeat(64)),
                &checkpoint('b'),
                &state('a'),
                &occurrence("run:one")
            )
            .expect_err("uppercase handle"),
        ProjectionError::InvalidHandle
    );
}

#[test]
fn key_debug_output_is_redacted() {
    let projection = key(6);
    let rendered = format!("{projection:?}");
    assert_eq!(rendered, "ProjectionKey(<redacted>)");
    assert!(!rendered.contains("0606"));
}
