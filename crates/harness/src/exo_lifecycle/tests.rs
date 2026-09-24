// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use crate as harness_api;
#[path = "../../tests/support/exo_lifecycle.rs"]
mod fixture;

use super::*;
use crate::provider_session::owner_journal::{CommitStage, inject_commit_failure};
use fixture::{Effect, Fixture};

#[path = "tests_history.rs"]
mod history;

#[path = "tests_derived_ids.rs"]
mod derived_ids;

#[path = "tests_identity_width.rs"]
mod identity_width;

#[test]
fn faults_at_every_intent_admitted_sent_boundary_never_hand_off_or_restore_permit() {
    for skip in 0..3 {
        for stage in [
            CommitStage::BeforeWrite,
            CommitStage::AfterPartialWrite,
            CommitStage::AfterFileSync,
            CommitStage::AfterRename,
            CommitStage::AfterDirectorySync,
        ] {
            let mut fixture = Fixture::new();
            let mut owner = fixture.owner();
            let mut effect = Effect::default();
            inject_commit_failure(Some(stage), skip);
            let result = owner.start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect,
            );
            inject_commit_failure(None, 0);
            assert!(matches!(result, Err(LifecycleError::Io)));
            assert_eq!(effect.calls, 0);
            assert!(matches!(
                owner.start(
                    fixture.manifest.clone(),
                    &fixture.input,
                    &mut fixture.store,
                    &fixture.fingerprint,
                    &mut effect
                ),
                Err(LifecycleError::Poisoned)
            ));
            drop(owner);
            let mut reopened = fixture.reopen().expect("authenticated restart");
            if skip == 2
                && matches!(
                    stage,
                    CommitStage::AfterRename | CommitStage::AfterDirectorySync
                )
            {
                assert_eq!(reopened.entries()[0].phase, LifecyclePhase::Unknown);
                assert!(reopened.entries()[0].possible_write);
            }
            assert!(
                reopened
                    .start(
                        fixture.manifest.clone(),
                        &fixture.input,
                        &mut fixture.store,
                        &fixture.fingerprint,
                        &mut effect
                    )
                    .is_err()
            );
            assert_eq!(effect.calls, 0);
            drop(reopened);
            let repeated = fixture.reopen().expect("repeated held restart");
            assert!(repeated.claim_epoch() >= 3);
        }
    }
}

#[test]
fn terminal_journal_failure_reuses_existing_result_without_another_effect() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let result = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start");
    let StartOutcome::Started(mut handle) = result else {
        panic!("fresh handle required");
    };
    inject_commit_failure(Some(CommitStage::BeforeWrite), 0);
    let result = owner.poll(&mut handle, &mut fixture.store);
    inject_commit_failure(None, 0);
    assert!(matches!(result, Err(LifecycleError::Io)));
    assert!(
        fixture
            .store
            .decision(&fixture.manifest.execution_id)
            .expect("stored")
            .completed
    );
    drop(owner);
    let mut reopened = fixture.reopen().expect("restart");
    assert!(
        reopened
            .reconcile_stored(&fixture.manifest, &fixture.input, &fixture.store)
            .is_ok()
    );
    assert_eq!(effect.calls, 1);
}

#[test]
fn actual_journal_schema_omits_private_input_output_and_owner_token() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let result = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start");
    let StartOutcome::Started(mut handle) = result else {
        panic!("fresh handle");
    };
    assert_private_schema(&serde_json::to_value(&owner.snapshot).expect("snapshot"));
    owner
        .poll(&mut handle, &mut fixture.store)
        .expect("complete");
    let value = serde_json::to_value(&owner.snapshot).expect("snapshot");
    assert_private_schema(&value);
    assert!(
        !serde_json::to_string(&value)
            .expect("json")
            .contains("owner-fixture")
    );
}

fn assert_private_schema(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                assert!(
                    ![
                        "input_bytes",
                        "result_payload",
                        "response",
                        "owner_token",
                        "observation",
                        "legal_action_ids"
                    ]
                    .contains(&name.as_str()),
                    "private field {name}"
                );
                assert_private_schema(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_private_schema(value);
            }
        }
        _ => (),
    }
}

#[test]
fn real_request_and_response_parsers_accept_exact_limits_and_reject_one_extra_byte() {
    let fixture = Fixture::new();
    let mut input = fixture.input.clone();
    input.resize(MAX_INPUT_BYTES, b' ');
    let mut manifest = fixture.manifest.clone();
    manifest.input_length = input.len();
    manifest.input_digest = crate::sha256_hex(&input);
    assert!(super::validation::input(&manifest, &input).is_ok());
    input.push(b' ');
    manifest.input_length = input.len();
    manifest.input_digest = crate::sha256_hex(&input);
    assert!(super::validation::input(&manifest, &input).is_err());
    let completion = fixture::Handle {
        ready: true,
        units: Some(3),
    }
    .poll()
    .expect("poll")
    .expect("completion");
    let mut response = completion.response;
    response.resize(8192, b' ');
    assert!(super::validation::response(&fixture.manifest, &fixture.input, &response).is_ok());
    response.push(b' ');
    assert!(super::validation::response(&fixture.manifest, &fixture.input, &response).is_err());
}

#[test]
fn revoked_before_admission_mutates_no_journal_or_store_and_calls_no_effect() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let before = std::fs::read(fixture.config.directory.join("journal.enc")).expect("journal");
    *fixture.authority.revoked.lock().expect("authority") = true;
    let mut effect = Effect::default();
    assert!(matches!(
        owner.start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect
        ),
        Err(LifecycleError::Fenced)
    ));
    assert_eq!(
        std::fs::read(fixture.config.directory.join("journal.enc")).expect("unchanged"),
        before
    );
    assert!(matches!(
        fixture.store.decision(&fixture.manifest.execution_id),
        Err(crate::ExecutionStoreError::Missing)
    ));
    assert_eq!(effect.calls, 0);
}

#[test]
fn prepared_intent_with_pending_decision_and_missing_reservation_stays_held() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    owner.snapshot.entries.push(LifecycleEntry::prepared(
        fixture.manifest.clone(),
        owner.claim_epoch(),
    ));
    owner.persist().expect("prepared intent");
    fixture
        .store
        .record_decision(&fixture.manifest.decision().expect("reference"))
        .expect("pending decision");
    drop(owner);
    let mut reopened = fixture.reopen().expect("restart");
    let mut effect = Effect::default();
    assert!(
        reopened
            .start(
                fixture.manifest.clone(),
                &fixture.input,
                &mut fixture.store,
                &fixture.fingerprint,
                &mut effect
            )
            .is_err()
    );
    assert_eq!(reopened.entries()[0].phase, LifecyclePhase::Prepared);
    assert!(matches!(
        fixture
            .store
            .provider_reservation(&fixture.manifest.reservation_id),
        Err(crate::ExecutionStoreError::Missing)
    ));
    assert_eq!(effect.calls, 0);
}

#[test]
fn pending_game_operation_blocks_provider_admission_without_redispatch() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let intent = crate::OperationIntent::new(
        fixture.manifest.lineage().expect("lineage"),
        "game-operation",
        &fixture.manifest.authority.state_id,
        fixture.manifest.authority.generation,
        "combat.end-turn",
        "payload-digest",
        "input-digest",
    )
    .expect("game intent");
    fixture
        .store
        .record_operation_intent(&intent)
        .expect("pending game operation");
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
    assert_eq!(
        fixture
            .store
            .pending_operations(&fixture.manifest.scope.episode_id)
            .expect("pending operation")
            .len(),
        1
    );
    assert_eq!(effect.calls, 0);
}
