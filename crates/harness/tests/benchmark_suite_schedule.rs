// SPDX-License-Identifier: MIT

#![allow(clippy::panic, clippy::unwrap_used, dead_code)]

// AC2: crash/retry/cancel during scheduling does not double-score a trial, erase an attempt or
// change the frozen seed corpus.

#[path = "benchmark_suite/fixtures.rs"]
mod fixtures;

use fixtures::{key_of, manifest};
use sts2_harness::benchmark_manifest::suite::{
    Metric, ScheduleError, Settlement, SuiteScheduler, TrialOutcome, TrialOutcomeError, TrialPhase,
    TrialResult,
};

fn victory(key: &str) -> TrialOutcome {
    TrialOutcome::completed(key, TrialResult::Victory)
        .recording(Metric::ActionCount, 9)
        .recording(Metric::CostMicros, 12)
}

#[test]
fn retry_and_replayed_settlement_never_double_score() {
    let suite = manifest();
    let key = key_of("case-alpha", "policy-exo", 0);
    let mut scheduler = SuiteScheduler::new(&suite).unwrap();

    assert_eq!(scheduler.start(&key).unwrap(), 1);
    assert_eq!(
        scheduler.start(&key).unwrap(),
        2,
        "a retry keeps its lineage"
    );
    assert_eq!(scheduler.phase_of(&key), Some(TrialPhase::Running));

    let outcome = victory(&key);
    assert_eq!(
        scheduler.settle(outcome.clone()).unwrap(),
        Settlement::Recorded
    );
    assert_eq!(scheduler.attempts(&key), Some(2));
    assert_eq!(scheduler.outcomes().len(), 1);

    assert_eq!(
        scheduler.settle(outcome).unwrap(),
        Settlement::Duplicate,
        "replaying the identical outcome is idempotent"
    );
    assert_eq!(
        scheduler.outcomes().len(),
        1,
        "a trial is scored exactly once"
    );

    let conflicting = TrialOutcome::completed(&key, TrialResult::Defeat);
    assert_eq!(
        scheduler.settle(conflicting).unwrap_err(),
        ScheduleError::ConflictingOutcome(key.clone())
    );
    assert_eq!(
        scheduler.start(&key).unwrap_err(),
        ScheduleError::AlreadyScored(key.clone())
    );
    assert_eq!(
        scheduler.cancel(&key).unwrap_err(),
        ScheduleError::AlreadyScored(key.clone())
    );
    assert_eq!(scheduler.outcomes()[0].result, Some(TrialResult::Victory));
}

#[test]
fn cancel_preserves_attempts_and_blocks_scoring() {
    let suite = manifest();
    let key = key_of("case-beta", "policy-local", 0);
    let mut scheduler = SuiteScheduler::new(&suite).unwrap();

    assert_eq!(scheduler.start(&key).unwrap(), 1);
    scheduler.cancel(&key).unwrap();
    assert!(
        scheduler.cancel(&key).is_ok(),
        "cancelling twice is idempotent"
    );
    assert_eq!(scheduler.phase_of(&key), Some(TrialPhase::Cancelled));
    assert_eq!(
        scheduler.attempts(&key),
        Some(1),
        "the attempt survives cancellation"
    );
    assert!(scheduler.outcomes().is_empty());

    assert_eq!(
        scheduler.settle(victory(&key)).unwrap_err(),
        ScheduleError::AlreadyCancelled(key.clone())
    );
    assert_eq!(
        scheduler.attempts(&key),
        Some(1),
        "settling cannot change a cancelled trial"
    );
    assert!(
        scheduler
            .pending()
            .iter()
            .all(|trial| trial.trial_key != key)
    );
}

#[test]
fn unknown_keys_and_malformed_outcomes_are_rejected() {
    let suite = manifest();
    let unknown = "suite/case/policy/7".to_owned();
    let mut scheduler = SuiteScheduler::new(&suite).unwrap();

    assert_eq!(
        scheduler.start(&unknown).unwrap_err(),
        ScheduleError::UnknownTrial(unknown.clone())
    );
    assert_eq!(
        scheduler.cancel(&unknown).unwrap_err(),
        ScheduleError::UnknownTrial(unknown.clone())
    );
    assert_eq!(
        scheduler
            .settle(TrialOutcome::cancelled(&unknown))
            .unwrap_err(),
        ScheduleError::UnknownTrial(unknown)
    );
    assert_eq!(
        scheduler.settle(TrialOutcome::cancelled("")).unwrap_err(),
        ScheduleError::InvalidOutcome(TrialOutcomeError::EmptyTrialKey)
    );
    assert_eq!(scheduler.planned_count(), 8);
}

#[test]
fn resume_rebuilds_progress_and_keeps_the_frozen_corpus() {
    let suite = manifest();
    let decided_key = key_of("case-alpha", "policy-exo", 0);
    let cancelled_key = key_of("case-beta", "policy-local", 1);
    let mut scheduler = SuiteScheduler::new(&suite).unwrap();

    assert_eq!(scheduler.start(&decided_key).unwrap(), 1);
    assert_eq!(scheduler.start(&decided_key).unwrap(), 2);
    let outcome = victory(&decided_key).with_attempts(2);
    scheduler.settle(outcome).unwrap();
    scheduler.start(&cancelled_key).unwrap();
    scheduler.cancel(&cancelled_key).unwrap();

    let recorded: Vec<TrialOutcome> = scheduler.outcomes().into_iter().cloned().collect();
    let resumed = SuiteScheduler::resume(&suite, recorded).unwrap();
    assert_eq!(resumed.suite_revision(), scheduler.suite_revision());
    assert_eq!(resumed.manifest().corpus, suite.corpus);
    assert_eq!(
        resumed.manifest().corpus.corpus_randomization_seed,
        444_000_222
    );
    assert_eq!(
        resumed.attempts(&decided_key),
        Some(2),
        "attempt lineage survives a restart"
    );
    assert_eq!(resumed.phase_of(&decided_key), Some(TrialPhase::Settled));
    assert_eq!(resumed.phase_of(&cancelled_key), Some(TrialPhase::Pending));
    assert_eq!(resumed.pending().len(), 7);
    assert!(!resumed.is_complete());
    assert_eq!(suite.digest().unwrap(), resumed.suite_revision());
}
