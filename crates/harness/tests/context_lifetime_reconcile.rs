// SPDX-License-Identifier: MIT

//! Reconciliation transition soundness and reload fidelity (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use fixture::{INSIDE, invocation, ledger_with, next_n_scope, scope_id};
use sts2_harness::context_control::{
    ContextLifetimeError, DispatchSettlement, DurableLifetimeState,
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
