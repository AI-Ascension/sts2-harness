// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::tests::{path, scope, store, valid_policy};
use super::*;
use std::sync::atomic::Ordering;

#[test]
fn adoption_generation_tracks_binding_changes_but_not_history_and_never_wraps() {
    let path = path("adoption-generation");
    let scope = scope();
    let owner = ProviderSessionPolicyOwner::open(
        store(&path, scope.clone()),
        scope.clone(),
        NativeCapabilities::fixture(),
    )
    .expect("owner");
    let policy_a = serde_json::to_vec(&valid_policy(scope.clone())).expect("policy A");
    let sha_a = owner.import(policy_a).expect("import A");
    owner.adopt_imported(&sha_a, 2).expect("adopt A");
    let (_, active_a, _, generation_a) = owner.active_with_generation().expect("active policy A");
    assert_eq!(active_a, sha_a);
    assert_eq!(generation_a, 1);

    let mut second = valid_policy(scope.clone());
    second.version = 2;
    second.epoch = 2;
    let sha_b = owner
        .import(serde_json::to_vec(&second).expect("policy B"))
        .expect("import B");
    assert_eq!(
        owner.active_with_generation().expect("unchanged active").3,
        generation_a,
        "history-only import must preserve the active generation"
    );
    owner.adopt_imported(&sha_b, 4).expect("adopt B");
    let (_, active_b, _, generation_b) = owner.active_with_generation().expect("active policy B");
    assert_eq!(active_b, sha_b);
    assert_eq!(generation_b, generation_a + 1);

    owner.adopt_imported(&sha_a, 5).expect("adopt A again");
    let (_, active_again, _, generation_again) = owner
        .active_with_generation()
        .expect("active policy A again");
    assert_eq!(active_again, sha_a);
    assert_eq!(generation_again, generation_b + 1);

    owner.adoption_generation.store(u64::MAX, Ordering::Relaxed);
    let revision = owner.metadata().expect("metadata").revision;
    assert!(matches!(
        owner.adopt_imported(&sha_b, revision),
        Err(ProviderSessionPolicyOwnerError::AdoptionGenerationExhausted)
    ));
    let (_, still_active, _, still_generation) = owner
        .active_with_generation()
        .expect("active policy remains unchanged");
    assert_eq!(still_active, sha_a);
    assert_eq!(still_generation, u64::MAX);
    assert_eq!(owner.metadata().expect("metadata").revision, revision);
    drop(owner);
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}
