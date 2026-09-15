// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use super::fixture::{Effect, Fixture, Handle};
use sts2_harness::exo_lifecycle::*;
use sts2_harness::*;

fn completed_store(fixture: &mut Fixture) {
    let completion = Handle {
        ready: true,
        units: Some(3),
    }
    .poll()
    .expect("poll")
    .expect("result");
    fixture
        .store
        .complete_provider_with_result(
            &fixture.manifest.reservation_id,
            &completion.result_ref,
            &sha256_hex(&completion.response),
            &completion.response,
            3,
        )
        .expect("commit result");
}

#[test]
fn completed_store_repairs_metadata_but_restarted_broker_stays_held() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(_handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    completed_store(&mut fixture);
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Sent);
    drop(owner);
    let mut restarted = fixture.reopen().expect("restart");
    assert_eq!(restarted.entries()[0].phase, LifecyclePhase::Unknown);
    let decision = restarted
        .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
        .expect("metadata repair");
    assert!(!decision.action_id.is_empty());
    assert_eq!(restarted.entries()[0].phase, LifecyclePhase::Completed);
    assert!(
        restarted
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn revoked_or_foreign_completed_result_cannot_be_released() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(_handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    completed_store(&mut fixture);
    drop(owner);
    let mut restarted = fixture.reopen().expect("restart");
    *fixture.authority.revoked.lock().expect("authority") = true;
    assert!(matches!(
        restarted.reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store),
        Err(LifecycleError::Fenced)
    ));
    assert_eq!(restarted.entries()[0].phase, LifecyclePhase::Unknown);
    *fixture.authority.revoked.lock().expect("authority") = false;
    let mut foreign = fixture.manifest.clone();
    foreign.reservation_id = "foreign-reservation".into();
    assert!(
        restarted
            .reconcile_stored(&foreign, &fixture.input, &fixture.store)
            .is_err()
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn sent_restart_keeps_reserved_charge_without_new_send() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(_handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    drop(owner);
    let mut restarted = fixture.reopen().expect("restart");
    assert_eq!(restarted.entries()[0].phase, LifecyclePhase::Unknown);
    assert!(
        restarted
            .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
            .is_err()
    );
    assert!(
        restarted
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .provider_reservation("reservation-1")
            .expect("charge")
            .state,
        ProviderReservationState::Reserved
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn unknown_store_cannot_be_forced_complete_by_late_evidence() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect {
        ambiguous: true,
        ..Effect::default()
    };
    assert!(
        owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect
            )
            .is_err()
    );
    let completion = Handle {
        ready: true,
        units: Some(3),
    }
    .poll()
    .expect("poll")
    .expect("result");
    assert!(
        fixture
            .store
            .complete_provider_with_result(
                "reservation-1",
                &completion.result_ref,
                &sha256_hex(&completion.response),
                &completion.response,
                3
            )
            .is_err()
    );
    assert!(
        owner
            .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .provider_reservation("reservation-1")
            .expect("charge")
            .state,
        ProviderReservationState::Unknown
    );
}

#[test]
fn pending_store_records_block_fresh_journal_and_zero_effects() {
    for reserve in [false, true] {
        let mut fixture = Fixture::new();
        let mut owner = fixture.owner();
        let lineage = ExecutionLineage::new(
            &fixture.config.scope.run_id,
            &fixture.config.scope.episode_id,
            &fixture.manifest.episode_attempt_id,
            &fixture.manifest.trajectory_id,
        )
        .expect("lineage");
        fixture
            .store
            .record_decision(
                &DecisionReference::new(
                    lineage.clone(),
                    "other-decision",
                    "other-input",
                    "revision",
                    "config",
                )
                .expect("decision"),
            )
            .expect("record");
        if reserve {
            fixture
                .store
                .reserve_provider(
                    &ProviderReservation::new(
                        lineage,
                        "other-reservation",
                        "other-decision",
                        "other-provider",
                        10,
                    )
                    .expect("reservation"),
                )
                .expect("reserve");
        }
        let mut effect = Effect::default();
        assert!(
            owner
                .start(
                    fixture.manifest.clone(),
                    &fixture.input,
                    &mut fixture.store,
                    &fixture.fingerprint,
                    &mut effect
                )
                .is_err()
        );
        assert!(owner.entries().is_empty());
        assert_eq!(effect.calls, 0);
    }
}

#[test]
fn deny_claim_or_invalid_key_never_creates_destination() {
    let mut fixture = Fixture::new();
    *fixture.authority.revoked.lock().expect("authority") = true;
    assert!(matches!(
        LifecycleOwner::create(
            fixture.config.clone(),
            [7; 32],
            fixture.broker.take().expect("broker"),
            "owner-fixture".into(),
            fixture.authority.clone()
        ),
        Err(LifecycleError::Fenced)
    ));
    assert!(!fixture.config.directory.exists());
    let mut zero = Fixture::new();
    assert!(
        LifecycleOwner::create(
            zero.config.clone(),
            [0; 32],
            zero.broker.take().expect("broker"),
            "owner-fixture".into(),
            zero.authority.clone()
        )
        .is_err()
    );
    assert!(!zero.config.directory.exists());
}

#[test]
fn completed_journal_does_not_release_missing_unknown_or_different_store_result() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    owner
        .poll(&mut handle, &mut fixture.store)
        .expect("complete");
    for state in 0..3 {
        let mut other = Fixture::new();
        if state > 0 {
            seed_reservation(&mut other);
            if state == 1 {
                other
                    .store
                    .mark_provider_unknown("reservation-1", ProviderFailureClass::Outage, None)
                    .expect("unknown");
            } else {
                let completion = Handle {
                    ready: true,
                    units: Some(3),
                }
                .poll()
                .expect("poll")
                .expect("result");
                other
                    .store
                    .complete_provider_with_result(
                        "reservation-1",
                        "different-result-ref",
                        &sha256_hex(&completion.response),
                        &completion.response,
                        3,
                    )
                    .expect("different result");
            }
        }
        assert!(
            owner
                .stored(&fixture.manifest, &fixture.input, &other.store)
                .is_err()
        );
        assert!(
            owner
                .reconcile_stored(&fixture.manifest, &fixture.input, &other.store)
                .is_err()
        );
    }
    assert_eq!(effect.calls, 1);
}

fn seed_reservation(fixture: &mut Fixture) {
    let manifest = &fixture.manifest;
    let lineage = ExecutionLineage::new(
        &manifest.scope.run_id,
        &manifest.scope.episode_id,
        &manifest.episode_attempt_id,
        &manifest.trajectory_id,
    )
    .expect("lineage");
    fixture
        .store
        .record_decision(
            &DecisionReference::new(
                lineage.clone(),
                &manifest.execution_id,
                &manifest.input_digest,
                &manifest.model_revision,
                &manifest.config_digest,
            )
            .expect("decision"),
        )
        .expect("record");
    fixture
        .store
        .reserve_provider(
            &ProviderReservation::new(
                lineage,
                &manifest.reservation_id,
                &manifest.execution_id,
                &manifest.provider_attempt_id,
                manifest.reserved_units,
            )
            .expect("reservation"),
        )
        .expect("reserve");
}
