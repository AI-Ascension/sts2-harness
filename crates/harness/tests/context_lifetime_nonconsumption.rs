// SPDX-License-Identifier: MIT

//! AC2 — previews, reloads and retries never consume, extend, or resurrect applicability
//! (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use fixture::{CEILING, INSIDE, invocation, ledger_with, next_n_scope, retry_of, scope_id};
use sts2_harness::context_control::{ContextLifetimeError, DispatchSettlement};

/// AC2: any number of previews and reloads leave consumption exactly where it was.
#[test]
fn repeated_previews_and_reloads_consume_nothing() {
    let ledger = ledger_with(next_n_scope(2));
    let baseline = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(baseline.consumed, 0);
    assert_eq!(baseline.remaining, 2);

    for _ in 0..64 {
        let again = ledger
            .preview(scope_id(), INSIDE)
            .expect("preview succeeds");
        assert_eq!(again, baseline, "a preview is a pure read");
    }
    assert!(
        ledger.manifests().is_empty(),
        "no preview minted a manifest"
    );
}

/// AC2: a transport retry carries the same logical invocation id and re-uses the original manifest
/// instead of consuming a second slot.
#[test]
fn a_transport_retry_reuses_its_original_admission() {
    let mut ledger = ledger_with(next_n_scope(2));
    let first = ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("first attempt admits");

    for attempt in 1..=5 {
        let replayed = ledger
            .admit(scope_id(), &retry_of("invocation-1", attempt), INSIDE, None)
            .expect("a retry of the same logical invocation is admitted idempotently");
        assert_eq!(replayed, first, "a retry returns the original manifest");
        assert_eq!(
            replayed.attempt, first.attempt,
            "a retry does not rewrite the recorded attempt"
        );
    }
    assert_eq!(ledger.manifests().len(), 1, "one slot, not six");
    let preview = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.consumed, 1);
    assert_eq!(preview.remaining, 1);
}

/// AC2: a retry cannot extend applicability, and it cannot resurrect it after the window closed.
#[test]
fn a_retry_neither_extends_nor_resurrects_applicability() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("first admits");
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("second admits");
    assert!(matches!(
        ledger.admit(scope_id(), &invocation("invocation-3"), INSIDE, None),
        Err(ContextLifetimeError::Exhausted { .. })
    ));

    // A retry of an already admitted invocation is still answered after exhaustion...
    let replayed = ledger
        .admit(scope_id(), &retry_of("invocation-1", 9), INSIDE, None)
        .expect("an idempotent replay is answered");
    assert_eq!(replayed.ordinal, 1);
    // ...but it does not widen the window: a genuinely new invocation is still refused.
    assert!(matches!(
        ledger.admit(scope_id(), &invocation("invocation-4"), INSIDE, None),
        Err(ContextLifetimeError::Exhausted { .. })
    ));

    // Past the ceiling, the replay is refused too: expiry is evaluated at admission, not at mint.
    assert!(matches!(
        ledger.admit(scope_id(), &retry_of("invocation-1", 10), CEILING, None),
        Err(ContextLifetimeError::Expired { .. })
    ));
    assert_eq!(ledger.manifests().len(), 2);
}

/// AC2: releasing a held invocation gives its slot back exactly once; a second reconciliation of
/// the same invocation does not hand back a second slot.
#[test]
fn release_restores_a_slot_exactly_once() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("first admits");
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("second admits");
    assert!(matches!(
        ledger.admit(scope_id(), &invocation("invocation-3"), INSIDE, None),
        Err(ContextLifetimeError::Exhausted { .. })
    ));

    ledger
        .reconcile("invocation-1", DispatchSettlement::Released)
        .expect("release succeeds");
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        1
    );
    ledger
        .reconcile("invocation-1", DispatchSettlement::Released)
        .expect("a repeated release is idempotent");
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        1,
        "a repeated release cannot free a second slot"
    );

    ledger
        .admit(scope_id(), &invocation("invocation-3"), INSIDE, None)
        .expect("the released slot admits a new invocation");
    assert!(matches!(
        ledger.admit(scope_id(), &invocation("invocation-4"), INSIDE, None),
        Err(ContextLifetimeError::Exhausted { .. })
    ));
}

/// AC2: a dispatched or still-unknown invocation keeps its slot. Only an explicit release returns
/// capacity, so an unknown outcome is never silently treated as "did not happen".
#[test]
fn held_and_dispatched_invocations_keep_their_slot() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admits");
    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admits");

    ledger
        .reconcile("invocation-1", DispatchSettlement::Held)
        .expect("reconcile as held");
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .remaining,
        0
    );
    ledger
        .reconcile("invocation-1", DispatchSettlement::Dispatched)
        .expect("reconcile as dispatched");
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .remaining,
        0,
        "a dispatched invocation keeps its slot"
    );
    assert!(matches!(
        ledger.admit(scope_id(), &invocation("invocation-3"), INSIDE, None),
        Err(ContextLifetimeError::Exhausted { .. })
    ));
}

/// AC2: reconciliation refuses an invocation the ledger never admitted, so a caller cannot free a
/// slot it does not hold.
#[test]
fn reconciliation_refuses_an_unheld_invocation() {
    let mut ledger = ledger_with(next_n_scope(2));
    assert_eq!(
        ledger.reconcile("never-admitted", DispatchSettlement::Released),
        Err(ContextLifetimeError::NotHeld {
            invocation_id: "never-admitted".to_owned()
        })
    );
}
