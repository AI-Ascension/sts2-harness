// SPDX-License-Identifier: MIT

#![allow(clippy::panic, clippy::unwrap_used, dead_code)]

// AC1: a 2-seed x 2-policy x 2-repetition synthetic suite produces exactly eight distinct logical
// trials with a complete result table and correctly isolated contexts.

#[path = "benchmark_suite/fixtures.rs"]
mod fixtures;

use std::collections::BTreeSet;

use fixtures::{key_of, manifest};
use sts2_harness::benchmark_manifest::suite::{
    CONTEXT_NAMESPACE_PREFIX, Metric, ReportError, SuiteManifestError, TrialOutcome, TrialResult,
    aggregate, ensure_metric_coverage, plan,
};

#[test]
fn two_by_two_by_two_plans_eight_distinct_isolated_trials() {
    let suite = manifest();
    let trials = plan(&suite).unwrap();
    assert_eq!(trials.len(), 8);
    assert_eq!(suite.planned_count().unwrap(), 8);
    let keys: BTreeSet<&str> = trials
        .iter()
        .map(|trial| trial.trial_key.as_str())
        .collect();
    assert_eq!(keys.len(), 8, "trial keys must be distinct");
    let namespaces: BTreeSet<&str> = trials
        .iter()
        .map(|trial| trial.context_namespace.as_str())
        .collect();
    assert_eq!(namespaces.len(), 8, "each trial needs its own context");
    for trial in &trials {
        assert_eq!(
            trial.context_namespace,
            format!("{CONTEXT_NAMESPACE_PREFIX}{}", trial.trial_key)
        );
    }
    let cells: BTreeSet<(&str, &str, u32)> = trials
        .iter()
        .map(|trial| {
            (
                trial.case_id.as_str(),
                trial.policy_id.as_str(),
                trial.repetition,
            )
        })
        .collect();
    assert_eq!(
        cells.len(),
        8,
        "every case/policy/repetition is covered once"
    );
    for repetition in 0..2 {
        assert!(cells.contains(&("case-alpha", "policy-exo", repetition)));
        assert!(cells.contains(&("case-beta", "policy-local", repetition)));
    }
}

fn decided(key: &str, result: TrialResult, latency: Option<u64>) -> TrialOutcome {
    let outcome = TrialOutcome::completed(key, result)
        .recording(Metric::ReachedFloor, 48)
        .recording(Metric::ActionCount, 13)
        .recording(Metric::ProviderTokens, 2_048)
        .recording(Metric::CostMicros, 731);
    match latency {
        Some(value) => outcome.recording(Metric::LatencyMillis, value),
        None => outcome,
    }
}

fn synthetic_outcomes() -> Vec<TrialOutcome> {
    vec![
        decided(
            &key_of("case-alpha", "policy-exo", 0),
            TrialResult::Victory,
            Some(120),
        ),
        decided(
            &key_of("case-alpha", "policy-exo", 1),
            TrialResult::Defeat,
            None,
        ),
        decided(
            &key_of("case-alpha", "policy-local", 0),
            TrialResult::Victory,
            None,
        ),
        decided(
            &key_of("case-alpha", "policy-local", 1),
            TrialResult::Victory,
            None,
        ),
        decided(
            &key_of("case-beta", "policy-exo", 0),
            TrialResult::Defeat,
            None,
        ),
        TrialOutcome::budget_censored(&key_of("case-beta", "policy-exo", 1)),
        TrialOutcome::infrastructure_failure(&key_of("case-beta", "policy-local", 0)),
        TrialOutcome::unknown_outcome(&key_of("case-beta", "policy-local", 1)),
    ]
}

#[test]
fn complete_result_table_reports_explicit_denominators() {
    let outcomes = synthetic_outcomes();
    let coverage = ensure_metric_coverage(&manifest(), &outcomes).unwrap();
    assert_eq!(coverage.planned_trials, 8);
    assert_eq!(coverage.recorded_trials, 8);
    assert_eq!(coverage.missing_trials, 0);
    let latency = coverage
        .metrics
        .iter()
        .find(|entry| entry.metric == "latency_millis")
        .unwrap();
    assert_eq!(latency.measured, 1);
    assert_eq!(latency.unavailable, 7, "unmeasured values are never zero");

    let report = aggregate(&manifest(), &outcomes).unwrap();
    assert_eq!(report.cells.len(), 4);
    assert_eq!(report.recorded_trials, 8);
    let alpha = report.cell("case-alpha", "policy-exo").unwrap();
    assert_eq!((alpha.planned, alpha.recorded, alpha.missing), (2, 2, 0));
    assert_eq!(
        (
            alpha.victories,
            alpha.defeats,
            alpha.censored,
            alpha.infrastructure_failures,
            alpha.unknown
        ),
        (1, 1, 0, 0, 0)
    );
    assert_eq!(alpha.decision_rate_ppm(), Some(1_000_000));
    let beta = report.cell("case-beta", "policy-local").unwrap();
    assert_eq!(beta.decided(), 0);
    assert_eq!(
        beta.decision_rate_ppm(),
        Some(0),
        "0 of 2 decided is not a missing rate"
    );
    assert_eq!((beta.infrastructure_failures, beta.unknown), (1, 1));
}

#[test]
fn coverage_rejects_unknown_metric_and_off_plan_or_duplicate_outcomes() {
    let mut unknown = manifest();
    unknown.metrics.push("hit_points".to_owned());
    assert_eq!(
        ensure_metric_coverage(&unknown, &[]).unwrap_err(),
        ReportError::UnknownMetric("hit_points".to_owned())
    );

    let outcomes = synthetic_outcomes();
    let mut off_plan = outcomes.clone();
    off_plan.push(TrialOutcome::cancelled("revision/case/policy/9"));
    assert!(matches!(
        ensure_metric_coverage(&manifest(), &off_plan).unwrap_err(),
        ReportError::UnplannedOutcome(_)
    ));

    let mut duplicated = outcomes.clone();
    duplicated.push(outcomes[0].clone());
    assert!(matches!(
        ensure_metric_coverage(&manifest(), &duplicated).unwrap_err(),
        ReportError::DuplicateOutcome(_)
    ));

    let mut missing = outcomes.clone();
    missing.pop();
    assert_eq!(
        ensure_metric_coverage(&manifest(), &missing)
            .unwrap()
            .missing_trials,
        1
    );

    let mut empty_cell = outcomes;
    empty_cell.retain(|outcome| !outcome.trial_key.contains("case-beta/policy-local"));
    let report = aggregate(&manifest(), &empty_cell).unwrap();
    let cell = report.cell("case-beta", "policy-local").unwrap();
    assert_eq!((cell.recorded, cell.missing), (0, 2));
    assert_eq!(
        cell.decision_rate_ppm(),
        None,
        "no denominator means no rate"
    );
}

#[test]
fn manifest_validation_and_revision_identity_are_stable() {
    let suite = manifest();
    assert!(suite.validate().is_ok());
    let revision = suite.digest().unwrap();
    assert_eq!(revision, manifest().digest().unwrap());
    assert_eq!(revision.len(), 64);

    let mut duplicate = suite.clone();
    duplicate.corpus.cases[1].case_id = duplicate.corpus.cases[0].case_id.clone();
    assert_eq!(
        duplicate.validate().unwrap_err(),
        SuiteManifestError::DuplicateCase
    );

    let mut no_repetitions = suite.clone();
    no_repetitions.repetitions = 0;
    assert_eq!(
        no_repetitions.validate().unwrap_err(),
        SuiteManifestError::InvalidRepetitions
    );

    let mut changed_corpus = suite.clone();
    changed_corpus.corpus.cases[0].native_game_seed += 1;
    assert_ne!(revision, changed_corpus.digest().unwrap());
}
