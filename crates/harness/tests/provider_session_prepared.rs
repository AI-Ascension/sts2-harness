// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::provider_session::*;

#[path = "support/provider_session_fixture.rs"]
mod fixture;

use fixture::{broker, held_binding, item, prepare, prepare_with};

#[test]
fn prepared_vector_revalidates_at_first_resumed_submission() {
    let mut drifted = broker();
    let drifted_binding = held_binding(&mut drifted);
    let prepared = prepare(&mut drifted, &drifted_binding.binding_id, "prepared-vector");
    drifted
        .explicit_resume("owner-fixture", &drifted_binding.binding_id)
        .expect("resume");
    // A history refresh advances the history epoch after approval; the exact approved bytes can no
    // longer be proven to belong to the current provider prefix.
    drifted
        .refresh_history(
            "owner-fixture",
            &drifted_binding.binding_id,
            "refresh-drift",
            vec![item(1)],
            1,
            false,
        )
        .expect("history refresh");
    assert_eq!(
        drifted.admit_turn(
            "owner-fixture",
            &drifted_binding.binding_id,
            &prepared.prepared_id,
            "turn-drift",
        ),
        Err(SessionError::Stale)
    );

    // The same input against an undrifted binding is still admitted, proving the fence tracks
    // real drift rather than refusing every turn.
    let mut stable = broker();
    let stable_binding = held_binding(&mut stable);
    let current = prepare(&mut stable, &stable_binding.binding_id, "prepared-current");
    stable
        .explicit_resume("owner-fixture", &stable_binding.binding_id)
        .expect("resume");
    stable
        .admit_turn(
            "owner-fixture",
            &stable_binding.binding_id,
            &current.prepared_id,
            "turn-current",
        )
        .expect("current admission");
}

#[test]
fn dependency_change_and_owner_rotation_deny_prepared_input() {
    let mut dependency = broker();
    let dependency_binding = held_binding(&mut dependency);
    dependency
        .set_dependencies(
            "owner-fixture",
            &dependency_binding.binding_id,
            vec!["source-1".to_owned()],
        )
        .expect("dependencies");
    let prepared = prepare_with(
        &mut dependency,
        &dependency_binding.binding_id,
        "prepared-dependency",
        vec!["source-1".to_owned()],
    );
    assert_eq!(prepared.dependency_ids, vec!["source-1"]);
    dependency
        .explicit_resume("owner-fixture", &dependency_binding.binding_id)
        .expect("resume");
    dependency
        .set_dependencies(
            "owner-fixture",
            &dependency_binding.binding_id,
            vec!["source-2".to_owned()],
        )
        .expect("dependency change");
    assert_eq!(
        dependency.admit_turn(
            "owner-fixture",
            &dependency_binding.binding_id,
            &prepared.prepared_id,
            "turn-dep",
        ),
        Err(SessionError::HeldRequired)
    );

    let mut rotating = broker();
    let rotating_binding = held_binding(&mut rotating);
    let rotating_prepared = prepare(
        &mut rotating,
        &rotating_binding.binding_id,
        "prepared-rotate",
    );
    rotating
        .explicit_resume("owner-fixture", &rotating_binding.binding_id)
        .expect("resume");
    rotating
        .replace_owner("owner-fixture", "replacement-owner")
        .expect("rotation");
    assert_eq!(
        rotating.admit_turn(
            "replacement-owner",
            &rotating_binding.binding_id,
            &rotating_prepared.prepared_id,
            "turn-rotate",
        ),
        Err(SessionError::HeldRequired)
    );
    assert_eq!(
        rotating.admit_turn(
            "owner-fixture",
            &rotating_binding.binding_id,
            &rotating_prepared.prepared_id,
            "turn-old-owner",
        ),
        Err(SessionError::Unauthorized)
    );
}
