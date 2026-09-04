// SPDX-License-Identifier: MIT

use sts2_harness::{EvaluationSample, Evaluator, TerminalOutcome};

#[test]
fn evaluation_reports_safety_quality_calibration_resources_progression_and_completion() {
    let mut evaluator = Evaluator::new(4).expect("capacity is valid");
    evaluator
        .observe(EvaluationSample {
            legal: true,
            verified: true,
            confidence_percent: Some(80),
            outcome_success: Some(true),
            request_bytes: 100,
            response_bytes: 200,
            latency_millis: 10,
            progressed: true,
            ..EvaluationSample::default()
        })
        .expect("sample is accepted");
    evaluator
        .observe(EvaluationSample {
            legal: false,
            stale: true,
            recovery_attempted: true,
            recovery_succeeded: true,
            regret_millis: Some(250),
            confidence_percent: Some(20),
            outcome_success: Some(false),
            request_bytes: 90,
            response_bytes: 120,
            latency_millis: 15,
            terminal: Some(TerminalOutcome::Defeat),
            ..EvaluationSample::default()
        })
        .expect("sample is accepted");

    let report = evaluator.report();
    assert_eq!(report.samples(), 2);
    assert_eq!(report.legal(), 1);
    assert_eq!(report.illegal(), 1);
    assert_eq!(report.stale(), 1);
    assert_eq!(report.verified(), 1);
    assert_eq!(report.unverified(), 1);
    assert_eq!(report.recovery_success_rate_millis(), 1000);
    assert_eq!(report.mean_regret_millis(), Some(250));
    assert_eq!(report.calibration_error_percent(), Some(20));
    assert_eq!(report.request_bytes(), 190);
    assert_eq!(report.response_bytes(), 320);
    assert_eq!(report.progression_steps(), 1);
    assert_eq!(report.defeats(), 1);
    assert!(!report.completed());
}

#[test]
fn evaluator_keeps_victory_and_defeat_separate_and_enforces_capacity() {
    let mut evaluator = Evaluator::new(1).expect("capacity is valid");
    evaluator
        .observe(EvaluationSample {
            terminal: Some(TerminalOutcome::Victory),
            ..EvaluationSample::default()
        })
        .expect("sample is accepted");
    assert!(evaluator.observe(EvaluationSample::default()).is_err());
    let report = evaluator.report();
    assert_eq!(report.victories(), 1);
    assert_eq!(report.defeats(), 0);
    assert!(report.completed());
}
