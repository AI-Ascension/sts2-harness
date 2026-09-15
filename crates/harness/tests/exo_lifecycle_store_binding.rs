// SPDX-License-Identifier: MIT

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::rc::Rc;
use sts2_harness as harness_api;
use sts2_harness::exo_lifecycle::*;
use sts2_harness::*;

#[path = "support/exo_lifecycle.rs"]
mod fixture;
use fixture::{Effect, Fixture, Handle};
#[path = "support/exo_lifecycle_store_fixture.rs"]
mod store_fixture;
use store_fixture::{CountedEffect, MutatingEffect, seed_foreign};

#[test]
fn foreign_stores_never_poll_or_mutate_and_original_handle_remains_usable() {
    for (exact, error, units) in [
        (false, false, Some(3)),
        (false, true, Some(3)),
        (false, false, None),
        (true, false, Some(3)),
    ] {
        let mut f = Fixture::new();
        let mut owner = f.owner();
        let polls = Rc::new(Cell::new(0));
        let mut effect = CountedEffect {
            polls: polls.clone(),
            error,
            units,
        };
        let StartOutcome::Started(mut handle) = owner
            .start(
                f.manifest.clone(),
                &f.input,
                &mut f.store,
                &f.fingerprint,
                &mut effect,
            )
            .expect("start")
        else {
            panic!("handle")
        };
        let (mut foreign, foreign_execution) = seed_foreign(&f, exact);
        let original_decision = f
            .store
            .decision(&f.manifest.execution_id)
            .expect("original");
        let original_charge = f
            .store
            .provider_reservation(&f.manifest.reservation_id)
            .expect("charge");
        let foreign_decision = foreign.decision(&foreign_execution).expect("foreign");
        let foreign_charge = foreign
            .provider_reservation(&f.manifest.reservation_id)
            .expect("foreign charge");
        let foreign_episode = foreign
            .load_episode(&foreign_charge.lineage.episode_id)
            .expect("episode");
        let original_episode = f
            .store
            .load_episode(&f.manifest.scope.episode_id)
            .expect("episode");
        let entries = owner.entries().to_vec();
        let journal = std::fs::read(f.config.directory.join("journal.enc")).expect("journal");
        assert!(matches!(
            owner.poll(&mut handle, &mut foreign),
            Err(LifecycleError::Held)
        ));
        assert_eq!(polls.get(), 0);
        assert_eq!(owner.entries(), entries);
        assert_eq!(
            std::fs::read(f.config.directory.join("journal.enc")).expect("journal"),
            journal
        );
        assert_eq!(
            f.store
                .decision(&f.manifest.execution_id)
                .expect("original"),
            original_decision
        );
        assert_eq!(
            f.store
                .provider_reservation(&f.manifest.reservation_id)
                .expect("charge"),
            original_charge
        );
        assert_eq!(
            foreign.decision(&foreign_execution).expect("foreign"),
            foreign_decision
        );
        assert_eq!(
            foreign
                .provider_reservation(&f.manifest.reservation_id)
                .expect("charge"),
            foreign_charge
        );
        assert_eq!(
            foreign
                .load_episode(&foreign_charge.lineage.episode_id)
                .expect("episode"),
            foreign_episode
        );
        assert_eq!(
            f.store
                .load_episode(&f.manifest.scope.episode_id)
                .expect("episode"),
            original_episode
        );
        let result = owner.poll(&mut handle, &mut f.store);
        assert_eq!(polls.get(), 1);
        if error || units.is_none() {
            assert!(result.is_err());
            assert_eq!(
                f.store
                    .provider_reservation(&f.manifest.reservation_id)
                    .expect("charge")
                    .state,
                ProviderReservationState::Unknown
            );
        } else {
            assert!(result.expect("original completion").is_some());
        }
        assert_eq!(
            foreign.decision(&foreign_execution).expect("foreign"),
            foreign_decision
        );
        assert_eq!(
            foreign
                .provider_reservation(&f.manifest.reservation_id)
                .expect("charge"),
            foreign_charge
        );
    }
}

#[test]
fn reopened_database_is_not_the_admitted_live_incarnation() {
    let mut f = Fixture::new();
    let mut owner = f.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            f.manifest.clone(),
            &f.input,
            &mut f.store,
            &f.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    let mut reopened = ExecutionStore::open(f.store.config().clone()).expect("same file");
    let before = reopened.decision(&f.manifest.execution_id).expect("before");
    assert!(matches!(
        owner.poll(&mut handle, &mut reopened),
        Err(LifecycleError::Held)
    ));
    assert_eq!(
        reopened.decision(&f.manifest.execution_id).expect("after"),
        before
    );
    assert!(
        owner
            .poll(&mut handle, &mut f.store)
            .expect("original")
            .is_some()
    );
}

fn complete(store: &mut ExecutionStore, manifest: &InvocationManifest) {
    let result = Handle {
        ready: true,
        units: Some(3),
    }
    .poll()
    .expect("poll")
    .expect("completion");
    store
        .complete_provider_with_result(
            &manifest.reservation_id,
            &result.result_ref,
            &sha256_hex(&result.response),
            &result.response,
            3,
        )
        .expect("store completion");
}

#[test]
fn live_result_reads_reject_clones_and_recovery_binds_one_exact_store() {
    let mut f = Fixture::new();
    let mut owner = f.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            f.manifest.clone(),
            &f.input,
            &mut f.store,
            &f.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    let (mut clone, _) = seed_foreign(&f, true);
    complete(&mut clone, &f.manifest);
    let journal = std::fs::read(f.config.directory.join("journal.enc")).expect("journal");
    assert!(
        owner
            .reconcile_stored(&f.manifest, &f.input, &clone)
            .is_err()
    );
    assert_eq!(
        std::fs::read(f.config.directory.join("journal.enc")).expect("journal"),
        journal
    );
    owner.poll(&mut handle, &mut f.store).expect("completion");
    assert!(owner.stored(&f.manifest, &f.input, &clone).is_err());
    assert!(
        owner
            .start(
                f.manifest.clone(),
                &f.input,
                &mut clone,
                &f.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert!(owner.stored(&f.manifest, &f.input, &f.store).is_ok());
    drop(owner);
    let mut restarted = f.reopen().expect("restart");
    restarted
        .reconcile_stored(&f.manifest, &f.input, &f.store)
        .expect("first recovery binding");
    let journal = std::fs::read(f.config.directory.join("journal.enc")).expect("journal");
    assert!(
        restarted
            .reconcile_stored(&f.manifest, &f.input, &clone)
            .is_err()
    );
    assert_eq!(
        std::fs::read(f.config.directory.join("journal.enc")).expect("journal"),
        journal
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn same_incarnation_mismatched_records_reject_before_poll_or_write() {
    for statement in [
        "UPDATE decisions SET input_fingerprint = 'changed-input'",
        "UPDATE decisions SET model_revision = 'changed-model'",
        "UPDATE decisions SET config_digest = 'changed-config'",
        "UPDATE decisions SET provider_reservation_id = 'changed-reservation'",
        "UPDATE provider_reservations SET provider_execution_id = 'changed-provider'",
        "UPDATE provider_reservations SET trajectory_id = 'changed-trajectory'",
        "UPDATE provider_reservations SET reserved_units = reserved_units + 1",
        "DELETE FROM decisions",
    ] {
        let mut f = Fixture::new();
        let mut owner = f.owner();
        let polls = Rc::new(Cell::new(0));
        let mut effect = CountedEffect {
            polls: polls.clone(),
            error: false,
            units: Some(3),
        };
        let StartOutcome::Started(mut handle) = owner
            .start(
                f.manifest.clone(),
                &f.input,
                &mut f.store,
                &f.fingerprint,
                &mut effect,
            )
            .expect("start")
        else {
            panic!("handle")
        };
        let database =
            rusqlite::Connection::open(&f.store.config().path).expect("synthetic corruption");
        // Inject damaged persisted references, including a missing decision row, independently
        // of the normal writer's foreign-key enforcement. The owner must reject these records.
        database
            .execute_batch("PRAGMA foreign_keys = OFF")
            .expect("enable synthetic reference corruption");
        database
            .execute_batch(statement)
            .expect("mutate synthetic record");
        let before = database_bytes(&f);
        let journal = std::fs::read(f.config.directory.join("journal.enc")).expect("journal");
        assert!(matches!(
            owner.poll(&mut handle, &mut f.store),
            Err(LifecycleError::Held)
        ));
        assert_eq!(polls.get(), 0);
        assert_eq!(database_bytes(&f), before);
        assert_eq!(
            std::fs::read(f.config.directory.join("journal.enc")).expect("journal"),
            journal
        );
        assert_eq!(owner.entries()[0].phase, LifecyclePhase::Sent);
    }
}

fn database_bytes(f: &Fixture) -> (Vec<u8>, Vec<u8>) {
    let path = &f.store.config().path;
    (
        std::fs::read(path).expect("database"),
        std::fs::read(format!("{}-wal", path.display())).expect("WAL"),
    )
}

#[test]
fn changed_identity_during_poll_never_completes_or_marks_another_charge_unknown() {
    for error in [false, true] {
        let mut f = Fixture::new();
        let mut owner = f.owner();
        let mut effect = MutatingEffect {
            path: f.store.config().path.clone(),
            error,
        };
        let StartOutcome::Started(mut handle) = owner
            .start(
                f.manifest.clone(),
                &f.input,
                &mut f.store,
                &f.fingerprint,
                &mut effect,
            )
            .expect("start")
        else {
            panic!("handle")
        };
        let before = f
            .store
            .decision(&f.manifest.execution_id)
            .expect("decision");
        let journal = std::fs::read(f.config.directory.join("journal.enc")).expect("journal");
        assert!(matches!(
            owner.poll(&mut handle, &mut f.store),
            Err(LifecycleError::Held)
        ));
        let charge = f
            .store
            .provider_reservation(&f.manifest.reservation_id)
            .expect("charge");
        assert_eq!(charge.provider_execution_id, "changed-during-poll");
        assert_eq!(charge.state, ProviderReservationState::Reserved);
        assert_eq!(charge.actual_units, None);
        assert_eq!(charge.failure, None);
        assert_eq!(
            f.store
                .decision(&f.manifest.execution_id)
                .expect("decision"),
            before
        );
        assert_eq!(
            std::fs::read(f.config.directory.join("journal.enc")).expect("journal"),
            journal
        );
        assert!(matches!(
            owner.poll(&mut handle, &mut f.store),
            Err(LifecycleError::Poisoned)
        ));
    }
}
