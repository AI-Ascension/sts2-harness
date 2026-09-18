// SPDX-License-Identifier: MIT

//! Reconciliation transition soundness and reload fidelity (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use fixture::{INSIDE, invocation, ledger_with, next_n_scope, scope_id};
use sts2_harness::context_control::{
    ContextLifetimeError, DispatchSettlement, DurableLifetimeState, MAX_LIFETIME_MANIFESTS,
};

/// A released slot may be refilled, and the refilled window must survive a reload.
///
/// `ordinal` is a monotonic admission index, so the third admission of a two-slot window is ordinal
/// 3; restore must not reject it for exceeding the declared capacity.
#[test]
fn a_released_slot_can_be_refilled_and_reloaded() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile("invocation-1", DispatchSettlement::Released)
        .expect("reconciliation succeeds");
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .remaining,
        1,
        "the release restored a slot"
    );
    ledger
        .admit(scope_id(), &invocation("invocation-3"), INSIDE, None)
        .expect("the freed slot can be refilled");
    let live = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(live.consumed, 2);

    let state = DurableLifetimeState::capture("run-1", &ledger);
    state.validate().expect("the image is valid");
    let restored = state.restore().expect("a refilled window must reload");
    assert_eq!(
        restored
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        live.consumed,
        "reload preserves the live count"
    );
    assert_eq!(restored.manifests().len(), 3);
}

/// A settled dispatch must not be un-consumed after the fact.
#[test]
fn a_dispatched_invocation_cannot_be_released() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile("invocation-1", DispatchSettlement::Dispatched)
        .expect("reconciliation succeeds");
    let before = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");

    let released = ledger.reconcile("invocation-1", DispatchSettlement::Released);
    assert!(
        released.is_err(),
        "a dispatched invocation must not return its slot, got {released:?}"
    );
    let after = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(after.consumed, before.consumed);
    assert_eq!(after.remaining, before.remaining);
}

/// A released invocation must not be re-settled as dispatched, which would desynchronise the live
/// count from the reloaded count.
#[test]
fn a_released_invocation_cannot_be_redispatched() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile("invocation-1", DispatchSettlement::Released)
        .expect("reconciliation succeeds");

    let redispatched = ledger.reconcile("invocation-1", DispatchSettlement::Dispatched);
    assert!(
        redispatched.is_err(),
        "a released invocation must not be re-settled, got {redispatched:?}"
    );
    let live = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    let restored = DurableLifetimeState::capture("run-1", &ledger)
        .restore()
        .expect("the ledger restores");
    assert_eq!(
        restored
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        live.consumed,
        "live and reloaded counts must agree"
    );
}

/// Re-settling to the same value is idempotent rather than an error, so a retried reconciliation
/// does not fail a caller that already settled.
#[test]
fn resettling_to_the_same_settlement_is_idempotent() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile("invocation-1", DispatchSettlement::Dispatched)
        .expect("reconciliation succeeds");
    let again = ledger
        .reconcile("invocation-1", DispatchSettlement::Dispatched)
        .expect("the same settlement is accepted again");
    assert_eq!(again.settlement, DispatchSettlement::Dispatched);
}

/// The restored window must not accept a manifest whose recorded scope revision disagrees with the
/// scope it is filed under, so provenance cannot be rewritten on reload.
#[test]
fn restore_refuses_a_manifest_claiming_a_foreign_scope_revision() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let mut state = DurableLifetimeState::capture("run-1", &ledger);
    state.manifests[0].scope_digest = "f".repeat(64);
    assert!(
        state.validate().is_err(),
        "a manifest whose scope revision disagrees with its scope must be refused"
    );
}

/// The number of scopes and manifests one run may hold is bounded, so a window cannot grow until it
/// is permanently unpersistable.
#[test]
fn scope_and_manifest_counts_are_bounded() {
    let mut ledger = ledger_with(next_n_scope(2));
    let mut issued = 1;
    loop {
        let extra = fixture::next_n_scope(2);
        let scope = sts2_harness::context_control::ContextLifetimeScope {
            scope_id: format!("scope-{}", issued + 1),
            ..extra
        };
        match ledger.issue(scope) {
            Ok(_) => issued += 1,
            Err(error) => {
                assert_eq!(error, ContextLifetimeError::InvalidInput);
                break;
            }
        }
        assert!(issued < 10_000, "the scope count must be bounded");
    }
    assert!(issued >= 2, "at least two scopes are allowed");
}

/// A window grown to its limit must still persist and reload.
///
/// The count bounds and the byte budget must agree: bounding only the counts let a full window
/// serialize past the state budget, so `persist_lifetime` refused a legally grown window forever.
#[test]
fn a_full_window_still_persists_and_reloads() {
    use std::fs;

    use sts2_harness::context_control::{
        ContextBoundary, ContextControlStore, ControlAuthority, StoreMode,
    };

    let mut ledger = ledger_with(next_n_scope(64));
    let mut made = 1usize;
    while made < 8 {
        let scope = sts2_harness::context_control::ContextLifetimeScope {
            scope_id: format!("s-{}", made + 1),
            ..next_n_scope(64)
        };
        if ledger.issue(scope).is_err() {
            break;
        }
        made += 1;
    }
    'grow: for index in 0..made {
        let sid = if index == 0 {
            scope_id().to_owned()
        } else {
            format!("s-{}", index + 1)
        };
        for item in 0..64 {
            let id = format!("{sid}-{item}");
            if ledger.admit(&sid, &invocation(&id), INSIDE, None).is_err() {
                break 'grow;
            }
        }
    }
    assert!(
        !ledger.manifests().is_empty(),
        "the window must have grown before the bound stopped it"
    );
    let expected = ledger.manifests().len();
    let state = DurableLifetimeState::capture("run-1", &ledger);
    let body = serde_json::to_vec(&state).expect("encode");
    assert!(
        body.len() <= 1024 * 1024,
        "a window reachable through the API must fit the state budget, got {} bytes",
        body.len()
    );

    let directory = std::env::temp_dir().join(format!("lifetime-full-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).expect("create fixture directory");
    let path = directory.join("control.sqlite3");
    let boundary = ContextBoundary {
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        agent_id: "agent-1".into(),
        state_id: "state-1".into(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "adapter-1".into(),
        model_revision: "model-1".into(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    };
    let authority = ControlAuthority::new(boundary, "revision-1");
    let mut store =
        ContextControlStore::create(&path, [0x7a; 32], "run-1", &authority, StoreMode::Enabled)
            .expect("create control store");
    store
        .persist_lifetime(&ledger)
        .expect("a window at its limit must persist");
    drop(store);

    let reopened = ContextControlStore::open(&path, [0x7a; 32], "run-1").expect("reopen store");
    let loaded = reopened
        .load_lifetime()
        .expect("load succeeds")
        .expect("state exists");
    let restored = loaded.restore().expect("a full window must restore");
    assert_eq!(restored.manifests().len(), expected);
}

/// An image that exceeds the manifest bound is refused by validation as well as by restore, so a
/// hand-built or corrupted image cannot smuggle in more history than the run may hold.
#[test]
fn validate_refuses_an_image_over_the_manifest_bound() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let mut state = DurableLifetimeState::capture("run-1", &ledger);
    let sample = state.manifests[0].clone();
    state.manifests = vec![sample; MAX_LIFETIME_MANIFESTS + 1];
    state.manifest_count = u32::try_from(state.manifests.len()).unwrap_or(u32::MAX);
    assert_eq!(
        state.validate(),
        Err(ContextLifetimeError::InvalidInput),
        "an oversized image must fail validation"
    );
    assert!(state.restore().is_err());
}

/// A long release/re-admit cycle with one slot permanently dispatched must keep the live and
/// reloaded counters in agreement, and the monotonic admission index must not rewind.
#[test]
fn repeated_release_readmit_cycles_stay_consistent_across_reload() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("keeper"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile("keeper", DispatchSettlement::Dispatched)
        .expect("reconciliation succeeds");
    for round in 0..5u32 {
        let id = format!("inv-{round}");
        ledger
            .admit(scope_id(), &invocation(&id), INSIDE, None)
            .expect("the freed slot can be refilled");
        ledger
            .reconcile(&id, DispatchSettlement::Released)
            .expect("reconciliation succeeds");
    }
    let live = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(
        live.consumed, 1,
        "only the permanently dispatched slot stays consumed"
    );
    assert_eq!(live.remaining, 1);

    let restored = DurableLifetimeState::capture("run-1", &ledger)
        .restore()
        .expect("a cycled window must reload");
    let after = restored
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(after.consumed, live.consumed);
    assert_eq!(after.remaining, live.remaining);
    assert!(after.held.is_empty());
    assert_eq!(restored.manifests().len(), ledger.manifests().len());
}

/// Settling to `Held` is a no-op, and a released invocation stays refused even after that no-op, so
/// the transition guard cannot be walked around.
#[test]
fn a_held_noop_does_not_reopen_a_released_invocation() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .reconcile("invocation-1", DispatchSettlement::Held)
        .expect("a held no-op is accepted");
    ledger
        .reconcile("invocation-1", DispatchSettlement::Released)
        .expect("reconciliation succeeds");
    assert!(
        ledger
            .reconcile("invocation-1", DispatchSettlement::Dispatched)
            .is_err(),
        "a released invocation must stay released"
    );
    let live = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    let restored = DurableLifetimeState::capture("run-1", &ledger)
        .restore()
        .expect("the ledger restores");
    assert_eq!(
        restored
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        live.consumed
    );
}

/// Reconciling an invocation that was never admitted is refused.
#[test]
fn reconcile_refuses_an_unknown_invocation() {
    let mut ledger = ledger_with(next_n_scope(2));
    assert!(
        ledger
            .reconcile("never-admitted", DispatchSettlement::Released)
            .is_err()
    );
}
