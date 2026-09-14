// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use crate as harness_api;
#[path = "../../tests/support/exo_lifecycle.rs"]
mod fixture;

use super::*;
use crate::provider_session::owner_journal::{CommitStage, inject_commit_failure};
use fixture::{Effect, Fixture};

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
