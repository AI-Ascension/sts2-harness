// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used, dead_code)]

// AC1: two children from one verified checkpoint produce independently stored outcomes, and
// same-start admission is checked for each trial.
// AC4: budget exhaustion, cancellation and a crash neither duplicate trials nor turn an unknown or
// partial outcome into a defeat.

#[path = "support/branch_experiment.rs"]
mod support;

use sts2_harness::ExactAssurance;
use sts2_harness::benchmark_manifest::branch_experiment::{
    AdmissionError, BranchExperimentError, BranchExperimentScheduler, BranchOutcome, ForkStrategy,
    MAX_BRANCH_LABEL_BYTES, MAX_CHILD_LABEL_BYTES, ScheduleError, Settlement, TrialPhase,
    admit_start, aggregate, plan,
};

use support::{
    alternate, checkpoint, child, completed, key, manifest, projection_key, state, trace,
};

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

// AC1: a declaration that validates is always settleable. A max-length child label must derive a
// trial key and context namespace inside MAX_BRANCH_LABEL_BYTES, and an over-long label must be
// refused at validate() rather than accepted and then rejected by every outcome check.
#[test]
fn a_max_length_child_label_validates_plans_and_settles() {
    let label = "c".repeat(MAX_CHILD_LABEL_BYTES);
    let experiment = manifest(
        ForkStrategy::AlternatePolicy,
        vec![child(&label, 'a', None), child("b", 'b', None)],
    );
    assert!(experiment.validate().is_ok());

    let revision = experiment.digest().expect("digest");
    let planned = plan(&experiment).expect("plan");
    let trial = planned
        .iter()
        .find(|trial| trial.child_label == label)
        .expect("planned trial");
    assert_eq!(trial.trial_key, format!("{revision}/{label}"));
    assert_eq!(
        trial.context_namespace,
        format!("branch-trial:{}", trial.trial_key)
    );
    assert!(trial.trial_key.len() <= MAX_BRANCH_LABEL_BYTES);
    assert!(trial.context_namespace.len() <= MAX_BRANCH_LABEL_BYTES);

    let mut scheduler = BranchExperimentScheduler::new(&experiment).expect("scheduler");
    assert_eq!(scheduler.start(&trial.trial_key).expect("start"), 1);
    let outcome = BranchOutcome::completed(&trial.trial_key, trace("action.v1", 0, 100, &["x"]))
        .with_start(ExactAssurance::RestoreVerified);
    assert!(outcome.validate().is_ok());
    assert_eq!(
        scheduler.settle(outcome).expect("settle"),
        Settlement::Recorded
    );

    let over_bound = manifest(
        ForkStrategy::AlternatePolicy,
        vec![child(&"c".repeat(MAX_CHILD_LABEL_BYTES + 1), 'a', None)],
    );
    assert_eq!(
        over_bound.validate(),
        Err(BranchExperimentError::InvalidLabel)
    );
    let at_label_bound = manifest(
        ForkStrategy::AlternatePolicy,
        vec![child(&"c".repeat(MAX_BRANCH_LABEL_BYTES), 'a', None)],
    );
    assert_eq!(
        at_label_bound.validate(),
        Err(BranchExperimentError::InvalidLabel)
    );
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
