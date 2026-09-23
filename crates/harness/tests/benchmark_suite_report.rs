// SPDX-License-Identifier: MIT

#![allow(clippy::panic, clippy::unwrap_used, dead_code)]

// AC4: golden reports exercise paired outcomes, missing pairs, censored trials, infrastructure
// failures and unavailable provider costs/revisions with explicit denominators.

#[path = "benchmark_suite/fixtures.rs"]
mod fixtures;

use fixtures::{key_of, manifest};
use sts2_harness::benchmark_manifest::suite::{
    Metric, ReportError, TrialOutcome, TrialResult, aggregate, compare_paired_policies,
    ensure_metric_coverage,
};

fn decided(key: &str, result: TrialResult) -> TrialOutcome {
    TrialOutcome::completed(key, result)
        .recording(Metric::ActionCount, 11)
        .recording(Metric::CostMicros, 250)
}

fn golden_outcomes() -> Vec<TrialOutcome> {
    vec![
        decided(&key_of("case-alpha", "policy-exo", 0), TrialResult::Victory),
        decided(&key_of("case-alpha", "policy-exo", 1), TrialResult::Defeat),
        decided(
            &key_of("case-alpha", "policy-local", 0),
            TrialResult::Victory,
        ),
        decided(
            &key_of("case-alpha", "policy-local", 1),
            TrialResult::Victory,
        ),
        decided(&key_of("case-beta", "policy-exo", 0), TrialResult::Defeat),
        TrialOutcome::budget_censored(&key_of("case-beta", "policy-exo", 1)),
        TrialOutcome::infrastructure_failure(&key_of("case-beta", "policy-local", 0)),
        TrialOutcome::unknown_outcome(&key_of("case-beta", "policy-local", 1)),
    ]
}

#[test]
fn golden_report_keeps_failures_and_costs_out_of_the_win_column() {
    let outcomes = golden_outcomes();
    let report = aggregate(&manifest(), &outcomes).unwrap();
    assert_eq!((report.planned_trials, report.recorded_trials), (8, 8));
    let alpha = report.cell("case-alpha", "policy-exo").unwrap();
    assert_eq!((alpha.victories, alpha.defeats), (1, 1));
    assert_eq!(alpha.decision_rate_ppm(), Some(1_000_000));
    let beta = report.cell("case-beta", "policy-exo").unwrap();
    assert_eq!((beta.defeats, beta.censored), (1, 1));
    assert_eq!(beta.decision_rate_ppm(), Some(500_000));
    let failed = report.cell("case-beta", "policy-local").unwrap();
    assert_eq!((failed.victories, failed.defeats), (0, 0));
    assert_eq!((failed.infrastructure_failures, failed.unknown), (1, 1));
    assert_eq!(failed.decision_rate_ppm(), Some(0));
    let cost = report
        .metrics
        .iter()
        .find(|metric| metric.metric == "cost_micros")
        .unwrap();
    assert_eq!(
        (cost.measured, cost.unavailable),
        (5, 3),
        "an unavailable cost is counted, never zero"
    );
    assert_eq!(report.evaluator_revision, "evaluator-2026-09-23");
}

#[test]
fn paired_comparison_reports_missing_pairs_and_honest_denominators() {
    let outcomes = vec![
        decided(&key_of("case-alpha", "policy-exo", 0), TrialResult::Victory),
        decided(&key_of("case-alpha", "policy-exo", 1), TrialResult::Victory),
        decided(
            &key_of("case-alpha", "policy-local", 0),
            TrialResult::Defeat,
        ),
        TrialOutcome::budget_censored(&key_of("case-alpha", "policy-local", 1)),
        TrialOutcome::infrastructure_failure(&key_of("case-beta", "policy-exo", 0)),
        TrialOutcome::budget_censored(&key_of("case-beta", "policy-exo", 1)),
        TrialOutcome::cancelled(&key_of("case-beta", "policy-local", 0)),
        TrialOutcome::unknown_outcome(&key_of("case-beta", "policy-local", 1)),
    ];
    let comparisons =
        compare_paired_policies(&manifest(), &outcomes, "policy-exo", "policy-local").unwrap();
    assert_eq!(comparisons.len(), 2);
    let alpha = comparisons
        .iter()
        .find(|c| c.case_id == "case-alpha")
        .unwrap();
    assert_eq!(
        (
            alpha.planned_pairs,
            alpha.paired,
            alpha.a_wins,
            alpha.b_wins
        ),
        (2, 1, 1, 0)
    );
    assert_eq!(
        (alpha.a_missing, alpha.b_missing, alpha.both_missing),
        (0, 1, 0)
    );
    assert_eq!(alpha.delta_win_rate_ppm(), Some(1_000_000));
    let beta = comparisons
        .iter()
        .find(|c| c.case_id == "case-beta")
        .unwrap();
    assert_eq!((beta.paired, beta.a_missing, beta.b_missing), (0, 2, 2));
    assert_eq!(beta.both_missing, 2);
    assert_eq!(
        beta.delta_win_rate_ppm(),
        None,
        "nothing paired, so there is no delta rather than a fabricated zero"
    );
}

#[test]
fn sanitized_report_omits_seeds_and_witnesses() {
    let outcomes = golden_outcomes();
    let report = aggregate(&manifest(), &outcomes).unwrap();
    let json = report.to_json_pretty().unwrap();
    for forbidden in [
        "987654321",
        "1234567890",
        "555000111",
        "444000222",
        "asc-state",
    ] {
        assert!(!json.contains(forbidden), "report leaked {forbidden}");
    }
    assert!(!json.contains("witness"));
    assert!(!json.contains("native_game_seed"));
    assert!(json.contains("case-alpha") && json.contains("policy-local"));
    assert!(json.contains(&report.suite_revision));
    assert_eq!(
        ensure_metric_coverage(&manifest(), &outcomes)
            .unwrap()
            .missing_trials,
        0
    );
}

#[test]
fn comparison_rejects_unknown_or_repeated_policies() {
    let outcomes = golden_outcomes();
    assert_eq!(
        compare_paired_policies(&manifest(), &outcomes, "policy-exo", "policy-exo").unwrap_err(),
        ReportError::RepeatedPolicy("policy-exo".to_owned())
    );
    assert_eq!(
        compare_paired_policies(&manifest(), &outcomes, "policy-exo", "policy-ghost").unwrap_err(),
        ReportError::UnknownPolicy("policy-ghost".to_owned())
    );
    assert_eq!(
        compare_paired_policies(&manifest(), &outcomes, "policy-exo", "policy-local")
            .unwrap()
            .len(),
        2
    );
}
