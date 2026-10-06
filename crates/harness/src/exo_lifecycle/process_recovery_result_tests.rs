// SPDX-License-Identifier: MIT

use super::fixture::{Effect, Fixture};
use crate::exo_lifecycle::{LifecycleError, StartOutcome};
use crate::provider_session::owner_journal::{CommitStage, inject_commit_failure};

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
