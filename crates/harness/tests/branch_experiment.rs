// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used, dead_code)]

// AC1: two children from one verified checkpoint produce independently stored outcomes, and
// same-start admission is checked for each trial.
// AC3: different legal first actions are identified at the correct ordinal; an identical endpoint
// does not erase an earlier divergence; incompatible and partial traces are labelled.
// AC4: budget exhaustion, cancellation and a crash neither duplicate trials nor turn an unknown or
// partial outcome into a defeat.
// AC5: a prefix-only start stays out of exact-restore statistics.
// AC6: the public export carries a keyed handle and no exact digest.

use sts2_harness::benchmark_manifest::branch_experiment::{
    AdmissionError, BRANCH_EXPERIMENT_VERSION, BranchBudgets, BranchDivergence,
    BranchExperimentError, BranchExperimentManifest, BranchExperimentScheduler, BranchOutcome,
    ChildPolicy, ForkStrategy, ScheduleError, Settlement, StopCondition, TrialPhase, admit_start,
    aggregate, compare_branches, plan,
};
use sts2_harness::{
    ExactAssurance, ExactCheckpointId, ExactCheckpointReference, ExactStateDigest, ProjectionKey,
    TransitionRecord, TransitionTrace,
};

fn state(value: u64) -> ExactStateDigest {
    ExactStateDigest::parse(&format!("asc-state:v1:sha256:{value:064x}")).expect("state digest")
}

fn checkpoint(assurance: ExactAssurance) -> ExactCheckpointReference {
    ExactCheckpointReference {
        exact_state_digest: state(0),
        exact_checkpoint_id: ExactCheckpointId::parse(&format!(
            "asc-checkpoint:v1:sha256:{}",
            "c".repeat(64)
        ))
        .expect("checkpoint id"),
        boundary_kind: String::from("decision"),
        boundary_phase: String::from("combat"),
        assurance,
    }
}

fn trace(profile: &str, source: u64, after_base: u64, actions: &[&str]) -> TransitionTrace {
    let source_state = state(source);
    let mut records = Vec::new();
    let mut before = source_state.clone();
    let mut previous: Option<String> = None;
    for (index, action) in actions.iter().enumerate() {
        let after = state(after_base + index as u64);
        let record = TransitionRecord {
            ordinal: index as u64,
            boundary_kind: String::from("decision"),
            boundary_phase: String::from("combat"),
            before: before.clone(),
            after: after.clone(),
            action_key: (*action).to_owned(),
            action_schema: String::from("action.v1"),
            catalog_witness: None,
            external_input_digest: None,
            previous_commitment: previous.clone(),
        };
        previous = Some(record.commitment().expect("commitment"));
        before = after;
        records.push(record);
    }
    TransitionTrace {
        profile: profile.to_owned(),
        source_state,
        records,
    }
}

fn settings(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn child(label: &str, seed: char, first_action: Option<&str>) -> ChildPolicy {
    ChildPolicy {
        child_label: label.to_owned(),
        settings_digest: settings(seed),
        first_action: first_action.map(str::to_owned),
    }
}

fn manifest(strategy: ForkStrategy, children: Vec<ChildPolicy>) -> BranchExperimentManifest {
    BranchExperimentManifest {
        version: BRANCH_EXPERIMENT_VERSION.to_owned(),
        experiment_id: String::from("exp-1"),
        benchmark_ref: String::from("bench-1"),
        fork_point: checkpoint(ExactAssurance::RestoreVerified),
        strategy,
        context_policy: String::from("context-default"),
        stop_conditions: vec![
            StopCondition::TerminalOutcome,
            StopCondition::BudgetExhausted,
        ],
        budgets: BranchBudgets {
            max_decisions_per_child: 10,
            max_total_decisions: 20,
            max_total_duration_millis: 60_000,
            max_provider_spend_micros: Some(1_000_000),
            max_concurrency: 1,
        },
        children,
    }
}

fn alternate(first: &str, second: &str) -> BranchExperimentManifest {
    manifest(
        ForkStrategy::AlternativeFirstAction,
        vec![child("a", 'a', Some(first)), child("b", 'b', Some(second))],
    )
}

fn key(manifest: &BranchExperimentManifest, label: &str) -> String {
    plan(manifest)
        .expect("plan")
        .into_iter()
        .find(|trial| trial.child_label == label)
        .expect("planned trial")
        .trial_key
}

fn completed(key: &str) -> BranchOutcome {
    BranchOutcome::completed(key, trace("action.v1", 0, 100, &["x"]))
        .with_start(ExactAssurance::RestoreVerified)
}

fn projection_key(seed: u8) -> ProjectionKey {
    ProjectionKey::new(&[seed; 32]).expect("projection key")
}

#[test]
fn planning_is_stable_and_each_child_gets_its_own_namespace() {
    let manifest = alternate("play_card:strike", "play_card:defend");
    let planned = plan(&manifest).expect("plan");
    assert_eq!(planned.len(), 2);
    assert_eq!(manifest.planned_count(), 2);
    assert_ne!(planned[0].trial_key, planned[1].trial_key);
    assert_ne!(planned[0].context_namespace, planned[1].context_namespace);
    assert_eq!(planned, plan(&manifest).expect("plan"));
    assert_eq!(manifest.digest().expect("digest").len(), 64);
}

#[test]
fn strategy_and_budget_declarations_are_checked() {
    let mut missing = alternate("a", "b");
    missing.children[1].first_action = None;
    assert_eq!(
        missing.validate(),
        Err(BranchExperimentError::AlternativeFirstActionRequired)
    );

    let mut policy = manifest(
        ForkStrategy::AlternatePolicy,
        vec![child("a", 'a', None), child("b", 'b', None)],
    );
    assert!(policy.validate().is_ok());
    policy.children[0].first_action = Some(String::from("play_card:strike"));
    assert_eq!(
        policy.validate(),
        Err(BranchExperimentError::UnexpectedFirstAction)
    );

    let mut duplicate = alternate("a", "b");
    duplicate.children[1].child_label = String::from("a");
    assert_eq!(
        duplicate.validate(),
        Err(BranchExperimentError::DuplicateChild)
    );

    let mut budget = alternate("a", "b");
    budget.budgets.max_total_decisions = 1;
    assert_eq!(budget.validate(), Err(BranchExperimentError::InvalidBudget));

    let mut digest = alternate("a", "b");
    digest.children[0].settings_digest = String::from("sha256:zz");
    assert_eq!(
        digest.validate(),
        Err(BranchExperimentError::InvalidSettingsDigest)
    );
}

#[test]
fn same_start_admission_accepts_only_the_shared_verified_fork_point() {
    let manifest = alternate("a", "b");
    let trial = plan(&manifest).expect("plan").remove(0);
    let admitted = admit_start(
        &manifest,
        &trial,
        &checkpoint(ExactAssurance::RestoreVerified),
    )
    .expect("admit");
    assert!(admitted.is_exact_restore());

    let mut other = checkpoint(ExactAssurance::RestoreVerified);
    other.exact_state_digest = state(9);
    assert_eq!(
        admit_start(&manifest, &trial, &other),
        Err(AdmissionError::StartMismatch)
    );
    assert_eq!(
        admit_start(&manifest, &trial, &checkpoint(ExactAssurance::CaptureOnly)),
        Err(AdmissionError::StartNotVerified)
    );
    assert_eq!(
        admit_start(
            &manifest,
            &trial,
            &checkpoint(ExactAssurance::PublicObservationOnly)
        ),
        Err(AdmissionError::StartNotVerified)
    );
}

#[test]
fn retry_is_idempotent_and_a_conflicting_settlement_is_refused() {
    let manifest = alternate("a", "b");
    let mut scheduler = BranchExperimentScheduler::new(&manifest).expect("scheduler");
    let a = key(&manifest, "a");
    assert_eq!(scheduler.start(&a).expect("start"), 1);
    let settled = completed(&a);
    assert_eq!(
        scheduler.settle(settled.clone()).expect("settle"),
        Settlement::Recorded
    );
    assert_eq!(
        scheduler.settle(settled).expect("settle"),
        Settlement::Duplicate
    );
    assert_eq!(
        scheduler.settle(BranchOutcome::cancelled(&a)),
        Err(ScheduleError::ConflictingOutcome(a.clone()))
    );
    assert_eq!(
        scheduler.start(&a),
        Err(ScheduleError::AlreadyScored(a.clone()))
    );
    assert_eq!(scheduler.phase_of(&a), Some(TrialPhase::Settled));
}

#[test]
fn a_settled_or_cancelled_trial_is_never_continued_as_a_new_start() {
    let manifest = alternate("a", "b");
    let mut scheduler = BranchExperimentScheduler::new(&manifest).expect("scheduler");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    assert_eq!(scheduler.start(&b).expect("start"), 1);
    scheduler.cancel(&b).expect("cancel");
    assert_eq!(scheduler.cancel(&b), Ok(()));
    assert_eq!(
        scheduler.start(&b),
        Err(ScheduleError::AlreadyCancelled(b.clone()))
    );
    assert_eq!(
        scheduler.settle(BranchOutcome::cancelled(&b)),
        Err(ScheduleError::AlreadyCancelled(b.clone()))
    );
    assert_eq!(scheduler.phase_of(&b), Some(TrialPhase::Cancelled));
    assert!(scheduler.pending().iter().all(|trial| trial.trial_key != b));
    assert!(!scheduler.is_complete());
    scheduler.cancel(&a).expect("cancel");
    assert!(scheduler.pending().is_empty());
    assert!(scheduler.is_complete());
}

#[test]
fn resume_replays_attempt_lineage_without_double_scoring() {
    let manifest = alternate("a", "b");
    let a = key(&manifest, "a");
    let mut started = BranchExperimentScheduler::new(&manifest).expect("scheduler");
    started.start(&a).expect("start");
    started.start(&a).expect("retry");
    let mut outcome = BranchOutcome::cancelled(&a);
    outcome.attempts = 2;
    started.settle(outcome.clone()).expect("settle");
    let resumed = BranchExperimentScheduler::resume(&manifest, [outcome]).expect("resume");
    assert_eq!(resumed.attempts(&a), Some(2));
    assert_eq!(resumed.phase_of(&a), Some(TrialPhase::Settled));
    assert_eq!(resumed.outcomes().len(), 1);
}

#[test]
fn cancel_and_budget_exhaustion_stay_out_of_a_defeat_tally() {
    let manifest = alternate("a", "b");
    let a = key(&manifest, "a");
    let b = key(&manifest, "b");
    let outcomes = vec![
        BranchOutcome::budget_censored(&a)
            .with_start(ExactAssurance::RestoreVerified)
            .with_trace(trace("action.v1", 0, 100, &["x"])),
        BranchOutcome::cancelled(&b),
    ];
    let report = aggregate(&manifest, &outcomes, &projection_key(7)).expect("aggregate");
    assert_eq!(report.settled, 0);
    assert_eq!(report.censored, 1);
    assert_eq!(report.cancelled, 1);
    assert_eq!(report.exact_restore_settled, 1);
    assert_eq!(report.restore_failures, 0);
}

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
