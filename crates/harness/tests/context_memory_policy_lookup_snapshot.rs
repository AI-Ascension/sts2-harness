// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;

use fixture::*;
use sts2_harness::context_memory::policy_owner::*;

fn assert_same_snapshot(before: &LookupPolicySnapshot, after: &LookupPolicySnapshot) {
    assert_eq!(before.binding, after.binding);
    assert_eq!(before.policy, after.policy);
    assert_eq!(before.capabilities, after.capabilities);
    assert_eq!(before.corpus.scope(), after.corpus.scope());
    assert_eq!(before.corpus.generation(), after.corpus.generation());
    assert_eq!(
        before.corpus.projection_generation(),
        after.corpus.projection_generation()
    );
    assert_eq!(
        before.corpus.revocation_epoch(),
        after.corpus.revocation_epoch()
    );
    assert_eq!(before.corpus.total_bytes(), after.corpus.total_bytes());
    assert_eq!(before.corpus.enabled(), after.corpus.enabled());
    assert_eq!(
        before.corpus.entries().collect::<Vec<_>>(),
        after.corpus.entries().collect::<Vec<_>>()
    );
}

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
fn selected_lookup_snapshot_rejects_clock_rollback_without_mutating_active_state() {
    let fixture = Fixture::new();
    fixture.adopt();

    let before = fixture
        .owner
        .lookup_snapshot(access(), None)
        .expect("active snapshot before rollback");
    let active_before = fixture
        .owner
        .inspect_active_binding(access())
        .expect("inspect active binding before rollback")
        .expect("active binding exists");
    let binding_bytes_before = serde_json::to_vec(&active_before).expect("serialize binding");
    let store_bytes_before = std::fs::read(&fixture.path).expect("read encrypted policy store");
    assert_eq!(before.binding, active_before);

    fixture
        .clock
        .0
        .store(99, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        fixture.owner.lookup_snapshot(access(), None),
        Err(PolicyOwnerError::GrantRevoked)
    ));
    let active_during_refusal = fixture
        .owner
        .inspect_active_binding(access())
        .expect("inspect active binding during refusal")
        .expect("active binding remains persisted");
    assert_eq!(active_during_refusal, active_before);
    assert_eq!(
        serde_json::to_vec(&active_during_refusal).expect("serialize binding during refusal"),
        binding_bytes_before
    );
    assert_eq!(
        std::fs::read(&fixture.path).expect("read policy store after refusal"),
        store_bytes_before
    );

    fixture
        .clock
        .0
        .store(100, std::sync::atomic::Ordering::SeqCst);
    let after = fixture
        .owner
        .lookup_snapshot(access(), None)
        .expect("lookup recovers when time returns to approval instant");
    assert_same_snapshot(&before, &after);
    assert_eq!(
        serde_json::to_vec(&after.binding).expect("serialize binding after recovery"),
        binding_bytes_before
    );
    assert_eq!(
        std::fs::read(&fixture.path).expect("read policy store after recovery"),
        store_bytes_before
    );
}

#[test]
fn selected_lookup_snapshot_rejects_expired_grant_without_mutating_store() {
    let fixture = Fixture::new();
    fixture.adopt();

    let before = fixture
        .owner
        .lookup_snapshot(access(), None)
        .expect("active snapshot before expiry");
    let active_before = fixture
        .owner
        .inspect_active_binding(access())
        .expect("inspect active binding before expiry")
        .expect("active binding exists");
    let binding_bytes_before = serde_json::to_vec(&active_before).expect("serialize binding");
    let store_bytes_before = std::fs::read(&fixture.path).expect("read encrypted policy store");
    assert_eq!(before.binding, active_before);

    fixture
        .clock
        .0
        .store(1000, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        fixture.owner.lookup_snapshot(access(), None),
        Err(PolicyOwnerError::GrantRevoked)
    ));
    assert_eq!(
        std::fs::read(&fixture.path).expect("read policy store after expiry refusal"),
        store_bytes_before
    );

    fixture
        .clock
        .0
        .store(100, std::sync::atomic::Ordering::SeqCst);
    let after = fixture
        .owner
        .lookup_snapshot(access(), None)
        .expect("lookup recovers when time returns before expiry");
    assert_same_snapshot(&before, &after);
    assert_eq!(after.binding, active_before);
    assert_eq!(
        serde_json::to_vec(&after.binding).expect("serialize binding after recovery"),
        binding_bytes_before
    );
    assert_eq!(
        std::fs::read(&fixture.path).expect("read policy store after expiry recovery"),
        store_bytes_before
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
