// SPDX-License-Identifier: MIT

use super::fixture::{self, Fixture};
use super::process_effect_fixture::{
    PersistentProcessEffect as PersistentEffect, expected_completion,
};
use crate::exo_lifecycle::LifecyclePhase;
use crate::{ExecutionStore, ProviderReservationState};
use std::fs;
use std::path::PathBuf;

pub(super) fn open_existing_fixture(root: PathBuf) -> Fixture {
    let manifest: super::super::InvocationManifest =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).expect("fixture manifest"))
            .expect("decode fixture manifest");
    let input = fs::read(root.join("input.bin")).expect("fixture input");
    assert_eq!(manifest.input_length, input.len());
    assert_eq!(manifest.input_digest, crate::sha256_hex(&input));
    let initial_broker = fixture::broker();
    let policy = initial_broker.policy().clone();
    let capabilities = initial_broker.capabilities().clone();
    let config = super::super::JournalConfig {
        directory: root.join("owner"),
        legacy_path: None,
        store_id: "journal-fixture".into(),
        scope: manifest.scope.clone(),
        owner_binding_digest: crate::sha256_hex("authenticated-owner-fixture"),
    };
    let store = ExecutionStore::open(crate::ExecutionStoreConfig::new(
        root.join("execution.sqlite3"),
    ))
    .expect("existing store");
    let fingerprint =
        crate::ExecutionFingerprint::new("seed", "build", "state", "config", "provider")
            .expect("fingerprint");
    Fixture {
        root,
        config,
        manifest,
        input,
        broker: None,
        policy,
        capabilities,
        authority: std::sync::Arc::new(fixture::Authority::default()),
        store,
        fingerprint,
        cleanup_on_drop: false,
    }
}

pub(super) fn assert_precommit_recovery(
    fixture: &mut Fixture,
    owner: &mut crate::exo_lifecycle::LifecycleOwner,
) {
    let decision = fixture
        .store
        .decision(&fixture.manifest.execution_id)
        .expect("decision row survives rollback");
    assert!(!decision.completed);
    assert!(!decision.unknown);
    assert!(decision.result_payload.is_none());
    let reservation = fixture
        .store
        .provider_reservation(&fixture.manifest.reservation_id)
        .expect("reservation row survives rollback");
    assert_eq!(reservation.state, ProviderReservationState::Reserved);
    assert_eq!(owner.entries().len(), 1);
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Unknown);
    assert!(owner.entries()[0].possible_write);
    assert!(
        owner
            .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
            .is_err()
    );
    let mut no_second_effect = PersistentEffect::new(&fixture.root);
    assert!(
        owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut no_second_effect,
            )
            .is_err()
    );
}

pub(super) fn assert_postcommit_recovery(
    fixture: &mut Fixture,
    owner: &mut crate::exo_lifecycle::LifecycleOwner,
) {
    let expected = expected_completion();
    let expected_digest = crate::sha256_hex(&expected.response);
    let stored = fixture
        .store
        .decision(&fixture.manifest.execution_id)
        .expect("durable decision");
    assert!(stored.completed);
    assert!(!stored.unknown);
    assert_eq!(
        stored.result_payload.as_deref(),
        Some(expected.response.as_slice())
    );
    assert_eq!(
        stored.reference.result_digest.as_deref(),
        Some(expected_digest.as_str())
    );
    let expected_decision =
        super::super::validation::response(&fixture.manifest, &fixture.input, &expected.response)
            .expect("expected exact decision");
    let recovered = owner
        .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
        .expect("recover exact result in new owner");
    assert_eq!(recovered, expected_decision);
    assert_eq!(owner.entries()[0].phase, LifecyclePhase::Completed);
    assert_eq!(
        owner.entries()[0].result_digest.as_deref(),
        Some(expected_digest.as_str())
    );
    let repeated = owner
        .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
        .expect("idempotent exact result recovery");
    assert_eq!(repeated, expected_decision);
    let mut no_second_effect = PersistentEffect::new(&fixture.root);
    assert!(
        owner
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut no_second_effect,
            )
            .is_err()
    );
}
