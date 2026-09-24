// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used, dead_code)]

// AC3: different legal first actions are identified at the correct ordinal; an identical endpoint
// does not erase an earlier divergence; incompatible and partial traces are labelled.
// AC5: a prefix-only start stays out of exact-restore statistics.
// AC6: the public export carries a keyed handle and no exact digest.

#[path = "support/branch_experiment.rs"]
mod support;

use sts2_harness::ExactAssurance;
use sts2_harness::ProjectionKey;
use sts2_harness::benchmark_manifest::branch_experiment::{
    BranchDivergence, BranchOutcome, ForkStrategy, aggregate, compare_branches,
};

use support::{alternate, child, completed, key, manifest, projection_key, trace};

#[test]
fn restore_failure_is_not_a_policy_result() {
    let manifest = alternate("a", "b");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::restore_failed(&a).with_start(ExactAssurance::RestoreSupported),
        completed(&b),
    ];
    let comparison = compare_branches(&manifest, &outcomes, "a", "b").expect("compare");
    assert_eq!(comparison.divergence, BranchDivergence::RestoreFailure);
    assert_eq!(comparison.first_divergence_ordinal, None);
}

#[test]
fn different_first_actions_are_policy_divergence_at_ordinal_zero() {
    let manifest = alternate("play_card:strike", "play_card:defend");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::completed(&a, trace("action.v1", 0, 100, &["play_card:strike"]))
            .with_start(ExactAssurance::RestoreVerified),
        BranchOutcome::completed(&b, trace("action.v1", 0, 100, &["play_card:defend"]))
            .with_start(ExactAssurance::RestoreVerified),
    ];
    let comparison = compare_branches(&manifest, &outcomes, "a", "b").expect("compare");
    assert_eq!(comparison.divergence, BranchDivergence::PolicyDivergence);
    assert_eq!(comparison.first_divergence_ordinal, Some(0));
    assert!(comparison.exact_restore);
    assert!(comparison.is_policy_divergence());
}

#[test]
fn an_identical_endpoint_does_not_erase_an_earlier_divergence() {
    let manifest = manifest(
        ForkStrategy::AlternatePolicy,
        vec![child("a", 'a', None), child("b", 'a', None)],
    );
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::completed(&a, trace("action.v1", 0, 100, &["x", "y"]))
            .with_start(ExactAssurance::RestoreVerified),
        BranchOutcome::completed(&b, trace("action.v1", 0, 100, &["x", "z"]))
            .with_start(ExactAssurance::RestoreVerified),
    ];
    let comparison = compare_branches(&manifest, &outcomes, "a", "b").expect("compare");
    assert_eq!(comparison.divergence, BranchDivergence::DifferentAction);
    assert_eq!(comparison.first_divergence_ordinal, Some(1));
    assert_eq!(comparison.last_equal_ordinal, Some(0));
}

// AC3: the per-pair comparison contract is order-independent. An unequal-length pair must not flip
// between MissingCapture and IdenticalOverRecordedRange with the operand order; it must surface the
// extra/unobserved boundary count instead.
#[test]
fn unequal_length_comparisons_are_order_independent_and_surface_unobserved_records() {
    let manifest = manifest(
        ForkStrategy::AlternatePolicy,
        vec![child("a", 'a', None), child("b", 'a', None)],
    );
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::completed(&a, trace("action.v1", 0, 100, &["x"]))
            .with_start(ExactAssurance::RestoreVerified),
        BranchOutcome::completed(&b, trace("action.v1", 0, 100, &["x", "y"]))
            .with_start(ExactAssurance::RestoreVerified),
    ];

    let a_then_b = compare_branches(&manifest, &outcomes, "a", "b").expect("compare a,b");
    let b_then_a = compare_branches(&manifest, &outcomes, "b", "a").expect("compare b,a");
    assert_eq!(
        a_then_b.divergence,
        BranchDivergence::IdenticalOverRecordedRange
    );
    assert_eq!(b_then_a.divergence, a_then_b.divergence);
    assert_eq!(a_then_b.unobserved_records, 1);
    assert_eq!(b_then_a.unobserved_records, a_then_b.unobserved_records);
    assert_eq!(a_then_b.first_divergence_ordinal, None);
    assert_eq!(
        b_then_a.first_divergence_ordinal,
        a_then_b.first_divergence_ordinal
    );
    assert_eq!(a_then_b.last_equal_ordinal, Some(0));
    assert_eq!(b_then_a.last_equal_ordinal, a_then_b.last_equal_ordinal);
    assert_eq!(a_then_b.compared_actions, b_then_a.compared_actions);
    assert!(a_then_b.exact_restore && b_then_a.exact_restore);

    // Equal-length identical traces still report zero unobserved boundaries in either order.
    let same = vec![completed(&a), completed(&b)];
    let forward = compare_branches(&manifest, &same, "a", "b").expect("compare a,b");
    let reverse = compare_branches(&manifest, &same, "b", "a").expect("compare b,a");
    assert_eq!(
        forward.divergence,
        BranchDivergence::IdenticalOverRecordedRange
    );
    assert_eq!(reverse.divergence, forward.divergence);
    assert_eq!(forward.unobserved_records, 0);
    assert_eq!(reverse.unobserved_records, 0);
}

#[test]
fn incompatible_and_partial_evidence_is_labelled_not_guessed() {
    let manifest = alternate("a", "b");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let only_a = vec![completed(&a)];
    let missing = compare_branches(&manifest, &only_a, "a", "b").expect("compare");
    assert_eq!(missing.divergence, BranchDivergence::MissingCapture);
    assert!(!missing.exact_restore);

    let outcomes = vec![
        BranchOutcome::completed(&a, trace("profile-1", 0, 100, &["x"]))
            .with_start(ExactAssurance::RestoreVerified),
        BranchOutcome::completed(&b, trace("profile-2", 0, 100, &["x"]))
            .with_start(ExactAssurance::RestoreVerified),
    ];
    let incompatible = compare_branches(&manifest, &outcomes, "a", "b").expect("compare");
    assert_eq!(incompatible.divergence, BranchDivergence::IncompatibleTrace);
    assert_eq!(
        compare_branches(&manifest, &outcomes, "a", "a"),
        Err(
            sts2_harness::benchmark_manifest::branch_experiment::ComparisonError::RepeatedChild(
                String::from("a")
            )
        )
    );
    assert!(matches!(
        compare_branches(&manifest, &outcomes, "a", "z"),
        Err(sts2_harness::benchmark_manifest::branch_experiment::ComparisonError::UnknownChild(_))
    ));
}

#[test]
fn a_prefix_only_start_is_excluded_from_exact_restore_statistics() {
    let manifest = alternate("a", "b");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::completed(&a, trace("action.v1", 0, 100, &["x"]))
            .with_start(ExactAssurance::PublicObservationOnly),
        completed(&b),
    ];
    assert!(!outcomes[0].exact_restore_eligible());
    let comparison = compare_branches(&manifest, &outcomes, "a", "b").expect("compare");
    assert!(!comparison.exact_restore);
    let report = aggregate(&manifest, &outcomes, &projection_key(7)).expect("aggregate");
    assert_eq!(report.settled, 2);
    assert_eq!(report.exact_restore_settled, 1);
}

#[test]
fn the_public_export_carries_a_keyed_handle_and_no_exact_digest() {
    let manifest = alternate("a", "b");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::completed(&a, trace("action.v1", 0, 100, &["x"]))
            .with_start(ExactAssurance::RestoreVerified),
        BranchOutcome::completed(&b, trace("action.v1", 0, 100, &["y"]))
            .with_start(ExactAssurance::RestoreVerified),
    ];
    let report = aggregate(&manifest, &outcomes, &projection_key(7)).expect("aggregate");
    let json = report.to_json_pretty().expect("json");
    assert!(report.experiment_ref.starts_with("ckpt-h1:"));
    assert_eq!(report.comparisons.len(), 1);
    for needle in ["asc-state", "asc-checkpoint", "sha256", "action.v1"] {
        assert!(!json.contains(needle), "public report leaked {needle}");
    }
    let other = aggregate(&manifest, &outcomes, &projection_key(9)).expect("aggregate");
    assert_ne!(report.experiment_ref, other.experiment_ref);
    assert!(ProjectionKey::new(&[0_u8; 8]).is_err());
}
