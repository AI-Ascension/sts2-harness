// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;

use fixture::*;
use sts2_harness::context_memory::policy_owner::*;

#[test]
fn selected_lookup_snapshot_is_owner_loaded_and_revalidated_against_current_fence() {
    let fixture = Fixture::new();
    fixture.adopt();

    let snapshot = fixture
        .owner
        .lookup_snapshot(access(), None)
        .expect("active owner lookup snapshot");
    assert_eq!(snapshot.policy.policy_id, "saved-policy");
    assert_eq!(snapshot.policy.version, 2);
    assert_eq!(snapshot.policy.scope, *snapshot.corpus.scope());
    assert_eq!(snapshot.capabilities, snapshot.corpus.capabilities());
    assert_eq!(snapshot.binding.target.policy_id, snapshot.policy.policy_id);
    fixture
        .owner
        .revalidate_lookup_snapshot(access(), &snapshot.binding)
        .expect("selected owner policy remains current");

    fixture
        .authority
        .update(|state| {
            state.control_epoch = state
                .control_epoch
                .checked_add(1)
                .ok_or(PolicyOwnerError::Capacity)?;
            Ok(())
        })
        .expect("publish owner control change");
    assert_eq!(
        fixture
            .owner
            .revalidate_lookup_snapshot(access(), &snapshot.binding),
        Err(PolicyOwnerError::StaleReview)
    );
}

#[test]
fn selected_lookup_snapshot_requires_authenticated_select_grant() {
    let fixture = Fixture::new();
    fixture.adopt();
    let denied = fixture.owner.lookup_snapshot(
        PolicyAccess {
            bearer: Some("other-token"),
            grant_id: "policy-grant",
        },
        None,
    );
    assert!(matches!(
        denied,
        Err(PolicyOwnerError::Unauthenticated | PolicyOwnerError::PermissionDenied)
    ));
}

#[test]
fn lookup_authority_guard_holds_current_policy_fence_until_boundary_release() {
    use std::sync::mpsc;
    use std::time::Duration;

    let fixture = Fixture::new();
    fixture.adopt();
    let snapshot = fixture
        .owner
        .lookup_snapshot(access(), None)
        .expect("active owner lookup snapshot");
    let guard = fixture
        .owner
        .lock_lookup_snapshot(access(), &snapshot.binding)
        .expect("linearized lookup authority");
    assert_eq!(guard.snapshot().binding, snapshot.binding);

    let authority = fixture.authority.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let update = std::thread::spawn(move || {
        started_tx.send(()).expect("signal update start");
        let result = authority.update(|state| {
            state.control_epoch = state
                .control_epoch
                .checked_add(1)
                .ok_or(PolicyOwnerError::Capacity)?;
            Ok(())
        });
        done_tx.send(result).expect("report update result");
    });

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("update thread started");
    assert!(done_rx.try_recv().is_err());
    drop(guard);
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("owner update proceeds after lifecycle boundary")
        .expect("owner update succeeds");
    update.join().expect("update thread exits");
    assert_eq!(
        fixture
            .owner
            .revalidate_lookup_snapshot(access(), &snapshot.binding),
        Err(PolicyOwnerError::StaleReview)
    );
}
