// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used, dead_code)]

// AC1: two serial trials have distinct process birth/instance identities against one unchanged
// baseline, and a mutated first-trial profile cannot leak into the second.
// AC2: reused live process, stale resume, mismatched baseline/build, shared destination and
// forged/stale readiness are all refused before action admission.
// AC3: failure injection before/after allocation, launch, seeded start and cleanup reconciles each
// operation without a duplicate effect or a rewritten baseline.
// AC4: cancellation and concurrent attempts keep lease isolation and completed evidence, and a
// cleanup failure is reported independently of the gameplay outcome.

#[path = "cold_launch/fixtures.rs"]
mod fixtures;

use fixtures::{birth, completed_trial, launched_trial, mutated_build, pristine};
use serde_json::{Value, json};
use sts2_harness::benchmark_manifest::cold_launch::{
    BaselineError, BaselineMismatch, ColdLaunchError, ColdLaunchOrchestrator, ColdLaunchStage,
    Destination, ProcessError, ReadinessProof, TrialLifecycle, evidence_of,
};

fn lease(id: &str, key: &str) -> Destination {
    Destination {
        destination_id: id.to_owned(),
        trial_key: key.to_owned(),
    }
}

fn illegal(result: Result<(), ColdLaunchError>) -> bool {
    matches!(result, Err(ColdLaunchError::IllegalTransition { .. }))
}

#[test]
fn serial_trials_get_distinct_births_against_one_unchanged_baseline() {
    let reference = pristine();
    let mut orchestrator = ColdLaunchOrchestrator::new(2).unwrap();
    let mut first = launched_trial(&mut orchestrator, "trial-one", "dest-one", "token-one", 1);
    let first_destination = first.destination().unwrap().destination_id.clone();
    first.record_stopped().unwrap();
    first.record_cleaned().unwrap();
    orchestrator.release_destination("dest-one").unwrap();
    orchestrator.retire_birth("token-one").unwrap();
    let mut second = launched_trial(&mut orchestrator, "trial-two", "dest-two", "token-two", 2);
    let second_destination = second.destination().unwrap().destination_id.clone();
    second.record_stopped().unwrap();
    second.record_cleaned().unwrap();
    orchestrator.release_destination("dest-two").unwrap();
    orchestrator.retire_birth("token-two").unwrap();

    let first_evidence = evidence_of(&first, &reference);
    let second_evidence = evidence_of(&second, &reference);
    assert_ne!(
        first_evidence.instance_generation,
        second_evidence.instance_generation
    );
    assert_ne!(first_evidence.birth_token, second_evidence.birth_token);
    assert_eq!(
        first_evidence.baseline_digest,
        second_evidence.baseline_digest
    );
    assert_eq!(
        first_evidence.baseline_digest, reference.baseline_digest,
        "each trial is provisioned from the immutable baseline"
    );
    assert!(first_evidence.mismatches.is_empty());
    assert!(second_evidence.mismatches.is_empty());
    assert_ne!(first_destination, second_destination);
    assert_eq!(first_evidence.destination_id, None);
    assert_eq!(second_evidence.destination_id, None);
    assert!(first.is_complete() && second.is_complete());
    assert_eq!(orchestrator.active_leases(), 0);
    assert_eq!(orchestrator.live_births(), 0);
}

#[test]
fn a_mutated_first_trial_baseline_cannot_leak_into_the_second() {
    let reference = pristine();
    let mut orchestrator = ColdLaunchOrchestrator::new(2).unwrap();
    let first = completed_trial(&mut orchestrator, "trial-one", "dest-one", "token-one", 1);

    let mut mutated = first.baseline().clone();
    mutated.profile.build_digest = "sha256:build-synthetic-9".to_owned();
    assert_eq!(
        mutated.compare(&reference),
        vec![BaselineMismatch::BuildDigest]
    );
    assert!(!mutated.is_compatible(&reference));

    let second = completed_trial(&mut orchestrator, "trial-two", "dest-two", "token-two", 2);
    assert_eq!(
        second.baseline(),
        &reference,
        "the pristine baseline is reused"
    );
    assert!(
        evidence_of(&second, &reference).mismatches.is_empty(),
        "the mutation of a cloned profile never reaches the next trial"
    );
}

#[test]
fn every_mismatch_category_is_refused_before_admission() {
    let reference = pristine();
    let mut by_digest = pristine();
    by_digest.baseline_digest = "sha256:baseline-other".to_owned();
    let mut by_version = pristine();
    by_version.profile.game_version = "9.9.9".to_owned();
    let mut by_profile = pristine();
    by_profile.profile.profile_id = "profile-other".to_owned();
    for (label, declared, reason) in [
        (
            "baseline digest",
            by_digest,
            BaselineMismatch::BaselineDigest,
        ),
        (
            "build digest",
            mutated_build(),
            BaselineMismatch::BuildDigest,
        ),
        ("game version", by_version, BaselineMismatch::GameVersion),
        ("profile id", by_profile, BaselineMismatch::ProfileId),
    ] {
        assert_eq!(
            TrialLifecycle::admit_against("trial-x", declared, &reference),
            Err(ColdLaunchError::BaselineMismatch {
                reasons: vec![reason]
            }),
            "{label} must be refused before admission"
        );
    }
    assert!(TrialLifecycle::admit_against("trial-x", pristine(), &reference).is_ok());
}

#[test]
fn a_reused_live_process_birth_is_refused() {
    let mut orchestrator = ColdLaunchOrchestrator::new(2).unwrap();
    let attested = birth("token-alpha", 1);
    orchestrator.attest_birth("trial-one", &attested).unwrap();
    orchestrator
        .attest_birth("trial-one", &attested)
        .expect("re-attesting for the same trial is idempotent");
    assert_eq!(orchestrator.live_births(), 1);
    assert_eq!(
        orchestrator.attest_birth("trial-two", &attested),
        Err(ColdLaunchError::ReusedLiveProcess {
            birth_token: "token-alpha".to_owned()
        })
    );
    assert_eq!(
        orchestrator.attest_birth("trial-two", &birth("", 1)),
        Err(ColdLaunchError::Process(ProcessError::InvalidToken))
    );
    assert!(orchestrator.retire_birth("token-alpha").is_some());
    orchestrator.attest_birth("trial-two", &attested).unwrap();
    assert_eq!(orchestrator.live_births(), 1);
}

#[test]
fn a_foreign_or_shared_destination_is_refused() {
    let mut trial = TrialLifecycle::admit("trial-one", pristine()).unwrap();
    assert_eq!(
        trial.reserve_destination(lease("dest-shared", "trial-two")),
        Err(ColdLaunchError::ForeignReconciliation {
            trial_key: "trial-two".to_owned()
        })
    );
    assert_eq!(trial.stage(), ColdLaunchStage::Admitted);
    assert!(trial.destination().is_none());

    let mut orchestrator = ColdLaunchOrchestrator::new(2).unwrap();
    orchestrator
        .lease_destination("dest-shared", "trial-one")
        .unwrap();
    assert_eq!(
        orchestrator.lease_destination("dest-shared", "trial-two"),
        Err(ColdLaunchError::DestinationLeased {
            destination_id: "dest-shared".to_owned(),
            trial_key: "trial-one".to_owned(),
        })
    );
}

#[test]
fn stale_readiness_and_a_lost_reply_are_refused_or_reconciled() {
    let reply = birth("token-adopt", 1);
    let mut adopted = TrialLifecycle::admit("trial-adopt", pristine()).unwrap();
    adopted
        .reserve_destination(lease("dest-adopt", "trial-adopt"))
        .unwrap();
    adopted.record_provisioned().unwrap();
    adopted.reconcile_lost_reply(&reply).unwrap();
    assert_eq!(adopted.stage(), ColdLaunchStage::Launched);
    assert_eq!(adopted.birth(), Some(&reply));

    let mut fresh = TrialLifecycle::admit("trial-fresh", pristine()).unwrap();
    assert_eq!(
        fresh.reconcile_lost_reply(&reply),
        Err(ColdLaunchError::ForeignReconciliation {
            trial_key: "trial-fresh".to_owned()
        }),
        "a controller restart cannot become a launch where none was authorized"
    );
    assert_eq!(fresh.stage(), ColdLaunchStage::Admitted);

    let mut trial = TrialLifecycle::admit("trial-stale", pristine()).unwrap();
    trial
        .reserve_destination(lease("dest-stale", "trial-stale"))
        .unwrap();
    trial.record_provisioned().unwrap();
    trial.record_launched(birth("token-stale", 1)).unwrap();
    assert_eq!(
        trial.record_ready(ReadinessProof::for_birth(
            birth("token-forged", 1),
            "token-readiness",
            1
        )),
        Err(ColdLaunchError::Process(ProcessError::StaleReadiness))
    );
    assert_eq!(
        trial.record_ready(ReadinessProof::for_birth(
            birth("token-stale", 1),
            "token-readiness",
            2
        )),
        Err(ColdLaunchError::Process(ProcessError::StaleReadiness))
    );
    assert_eq!(
        trial.record_ready(ReadinessProof::for_birth(birth("token-stale", 1), "", 1)),
        Err(ColdLaunchError::Process(ProcessError::InvalidToken))
    );
    assert_eq!(trial.stage(), ColdLaunchStage::Launched);
    assert!(trial.readiness().is_none());

    let attested = trial.birth().unwrap().clone();
    trial.reconcile_lost_reply(&attested).unwrap();
    trial
        .reconcile_lost_reply(&attested)
        .expect("a reply for the recorded birth is idempotent");
    assert_eq!(
        trial.reconcile_lost_reply(&birth("token-other", 1)),
        Err(ColdLaunchError::ForeignReconciliation {
            trial_key: "trial-stale".to_owned()
        })
    );
}

#[test]
fn failure_injection_at_each_stage_changes_nothing() {
    let mut trial = TrialLifecycle::admit("trial-inject", pristine()).unwrap();
    assert!(illegal(trial.record_provisioned()));
    assert!(illegal(trial.record_launched(birth("token-inject", 1))));
    assert_eq!(trial.stage(), ColdLaunchStage::Admitted);

    trial
        .reserve_destination(lease("dest-inject", "trial-inject"))
        .unwrap();
    assert!(illegal(trial.record_launched(birth("token-inject", 1))));
    assert_eq!(trial.stage(), ColdLaunchStage::DestinationReserved);
    trial.record_provisioned().unwrap();

    trial.record_launched(birth("token-inject", 1)).unwrap();
    assert!(illegal(trial.record_launched(birth("token-inject", 1))));
    assert!(illegal(trial.record_setup_settled()));
    assert_eq!(trial.stage(), ColdLaunchStage::Launched);
    assert_eq!(trial.birth().map(|b| b.instance_generation), Some(1));

    trial
        .record_ready(ReadinessProof::for_birth(
            birth("token-inject", 1),
            "token-readiness",
            1,
        ))
        .unwrap();
    assert!(illegal(trial.record_running()));
    assert!(illegal(trial.record_cleaned()));
    assert_eq!(trial.stage(), ColdLaunchStage::Ready);
    assert_eq!(
        trial.baseline().baseline_digest,
        pristine().baseline_digest,
        "no failed transition rewrites or deletes the baseline"
    );
}

#[test]
fn cleanup_failure_is_distinct_from_the_gameplay_outcome() {
    let reference = pristine();
    let mut orchestrator = ColdLaunchOrchestrator::new(4).unwrap();
    let winner = completed_trial(
        &mut orchestrator,
        "trial-winner",
        "dest-winner",
        "t-winner",
        1,
    );
    let mut loser = launched_trial(&mut orchestrator, "trial-loser", "dest-loser", "t-loser", 2);
    loser.record_stopped().unwrap();
    loser.record_cleanup_failed().unwrap();

    let winner_evidence = evidence_of(&winner, &reference);
    let loser_evidence = evidence_of(&loser, &reference);
    assert!(!winner_evidence.cleanup_failed && winner.is_complete());
    assert!(loser_evidence.cleanup_failed && !loser.is_complete());
    assert!(loser_evidence.cleanup_failed);
    assert!(illegal(loser.record_cleaned()));
    assert_eq!(
        loser.destination().map(|d| d.destination_id.as_str()),
        Some("dest-loser")
    );

    orchestrator.quarantine_destination("dest-loser", "trial-loser");
    assert_eq!(
        orchestrator.lease_destination("dest-loser", "trial-other"),
        Err(ColdLaunchError::DestinationQuarantined(
            "trial-loser".to_owned()
        ))
    );
}

#[test]
fn cancellation_quarantines_the_destination_and_keeps_evidence() {
    let mut orchestrator = ColdLaunchOrchestrator::new(4).unwrap();
    let mut trial = launched_trial(&mut orchestrator, "trial-cancel", "dest-cancel", "t-can", 3);
    trial.quarantine().unwrap();
    assert_eq!(trial.stage(), ColdLaunchStage::Quarantined);
    assert!(illegal(trial.quarantine()));
    let evidence = evidence_of(&trial, &pristine());
    assert_eq!(evidence.stage, ColdLaunchStage::Quarantined);
    assert_eq!(evidence.instance_generation, Some(3));
    assert_eq!(evidence.birth_token.as_deref(), Some("t-can"));
    orchestrator.quarantine_destination("dest-cancel", "trial-cancel");
    assert_eq!(orchestrator.active_leases(), 0);
    assert_eq!(
        orchestrator.lease_destination("dest-cancel", "trial-other"),
        Err(ColdLaunchError::DestinationQuarantined(
            "trial-cancel".to_owned()
        ))
    );
}

#[test]
fn bounded_declarations_and_tokens_are_refused() {
    let mut baseline = pristine();
    baseline.baseline_digest.clear();
    assert_eq!(baseline.validate(), Err(BaselineError::InvalidLabel));
    let mut baseline = pristine();
    baseline.profile.game_version = "v".repeat(257);
    assert_eq!(baseline.validate(), Err(BaselineError::InvalidLabel));
    let mut baseline = pristine();
    baseline.exclusions.paths = (0..33).map(|n| format!("telemetry/{n}.log")).collect();
    assert_eq!(baseline.validate(), Err(BaselineError::TooManyExclusions));
    let mut baseline = pristine();
    baseline.exclusions.paths = vec!["telemetry/\0bad.log".to_owned()];
    assert_eq!(baseline.validate(), Err(BaselineError::InvalidLabel));

    assert_eq!(
        TrialLifecycle::admit("", pristine()).unwrap_err(),
        ColdLaunchError::InvalidTrialKey
    );
    assert_eq!(
        TrialLifecycle::admit(&"k".repeat(257), pristine()).unwrap_err(),
        ColdLaunchError::InvalidTrialKey
    );
    assert_eq!(birth("", 1).validate(), Err(ProcessError::InvalidToken));
    assert_eq!(
        birth("token", 0).validate(),
        Err(ProcessError::ZeroGeneration)
    );
    assert_eq!(
        ReadinessProof::for_birth(birth("token", 1), "", 1).validate(),
        Err(ProcessError::InvalidToken)
    );
}

#[test]
fn evidence_is_machine_readable_and_stage_labels_are_distinct() {
    let mut orchestrator = ColdLaunchOrchestrator::new(2).unwrap();
    let trial = completed_trial(&mut orchestrator, "trial-evidence", "dest-ev", "t-ev", 7);
    let evidence = evidence_of(&trial, &pristine());
    let value: Value = serde_json::to_value(&evidence).unwrap();
    assert_eq!(value["trial_key"], "trial-evidence");
    assert_eq!(value["stage"], "cleaned");
    assert_eq!(value["instance_generation"], 7);
    assert_eq!(value["birth_token"], "t-ev");
    assert_eq!(value["destination_id"], Value::Null);
    assert_eq!(value["cleanup_failed"], false);
    assert_eq!(value["mismatches"], json!([]));
    assert_eq!(value["baseline_digest"], "sha256:baseline-synthetic-0001");

    let stages = [
        ColdLaunchStage::Admitted,
        ColdLaunchStage::DestinationReserved,
        ColdLaunchStage::Provisioned,
        ColdLaunchStage::Launched,
        ColdLaunchStage::Ready,
        ColdLaunchStage::SetupSettled,
        ColdLaunchStage::Running,
        ColdLaunchStage::Stopped,
        ColdLaunchStage::Cleaned,
        ColdLaunchStage::CleanupFailed,
        ColdLaunchStage::Quarantined,
    ];
    let mut labels: Vec<&str> = stages.iter().map(|stage| stage.label()).collect();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), stages.len());
    assert_eq!(
        ColdLaunchStage::DestinationReserved.label(),
        "destination_reserved"
    );
}
