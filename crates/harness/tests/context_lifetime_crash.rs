// SPDX-License-Identifier: MIT

//! AC3 — crash tests on both sides of durable admission preserve counts, and an unknown possible
//! dispatch stays consumed and held until reconciliation (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use fixture::{INSIDE, invocation, ledger_with, next_n_scope, retry_of, scope_id};
use sts2_harness::context_control::{ContextLifetimeError, DispatchSettlement, LifetimeFailpoint};

/// AC3: a crash *before* durable admission preserves the count exactly — nothing was consumed.
#[test]
fn crash_before_admission_preserves_the_count() {
    let mut ledger = ledger_with(next_n_scope(3));
    for _ in 0..5 {
        let stopped = ledger.admit(
            scope_id(),
            &invocation("invocation-1"),
            INSIDE,
            Some(LifetimeFailpoint::BeforeDurableAdmission),
        );
        assert_eq!(
            stopped,
            Err(ContextLifetimeError::InterruptedBeforeAdmission)
        );
    }
    assert!(ledger.manifests().is_empty());
    let preview = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.consumed, 0);
    assert_eq!(preview.remaining, 3);
    assert!(preview.held.is_empty());

    // The same invocation then admits normally, proving the failure consumed no reservation.
    let manifest = ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds after the pre-durable crash");
    assert_eq!(manifest.ordinal, 1);
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .remaining,
        2
    );
}

/// AC3: a crash *after* durable admission keeps the slot consumed and the invocation held, so a
/// dispatch that may have happened is never silently handed back.
#[test]
fn crash_after_admission_keeps_the_slot_consumed_and_held() {
    let mut ledger = ledger_with(next_n_scope(3));
    let stopped = ledger.admit(
        scope_id(),
        &invocation("invocation-1"),
        INSIDE,
        Some(LifetimeFailpoint::AfterDurableAdmission),
    );
    assert_eq!(
        stopped,
        Err(ContextLifetimeError::InterruptedAfterAdmission)
    );

    let preview = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.consumed, 1, "the slot stays consumed");
    assert_eq!(preview.remaining, 2);
    assert_eq!(
        preview.held,
        vec!["invocation-1".to_owned()],
        "the invocation stays held as a possible dispatch"
    );
    assert_eq!(ledger.manifests().len(), 1);
}

/// AC3: after the post-durable crash, a retry of the *same* logical invocation is answered with the
/// held manifest rather than consuming a second slot.
#[test]
fn post_crash_retry_reuses_the_held_manifest() {
    let mut ledger = ledger_with(next_n_scope(3));
    let _ = ledger.admit(
        scope_id(),
        &invocation("invocation-1"),
        INSIDE,
        Some(LifetimeFailpoint::AfterDurableAdmission),
    );
    let recovered = ledger
        .admit(scope_id(), &retry_of("invocation-1", 1), INSIDE, None)
        .expect("the retry is answered idempotently");
    assert_eq!(recovered.ordinal, 1);
    assert_eq!(ledger.manifests().len(), 1);
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .consumed,
        1
    );
}

/// AC3: reconciliation settles the held invocation. Releasing it restores capacity; the historical
/// manifest is mutated only in its settlement field and is never deleted.
#[test]
fn reconciliation_settles_the_held_invocation() {
    let mut ledger = ledger_with(next_n_scope(2));
    let _ = ledger.admit(
        scope_id(),
        &invocation("invocation-1"),
        INSIDE,
        Some(LifetimeFailpoint::AfterDurableAdmission),
    );
    assert_eq!(
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview")
            .remaining,
        1
    );

    let settled = ledger
        .reconcile("invocation-1", DispatchSettlement::Released)
        .expect("reconciliation succeeds");
    assert_eq!(settled.settlement, DispatchSettlement::Released);
    assert_eq!(settled.ordinal, 1, "the record is not rewritten");
    assert_eq!(settled.manifest_id, "scope-1#1");
    let preview = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.remaining, 2);
    assert!(preview.held.is_empty());
    assert_eq!(ledger.manifests().len(), 1, "history is retained");
}

/// AC3: settlement never rewrites the record's identity or its canonical bytes, so an earlier
/// admission stays auditable under retention policy.
#[test]
fn settlement_does_not_rewrite_the_admitted_record() {
    let mut ledger = ledger_with(next_n_scope(2));
    let admitted = ledger
        .admit(
            scope_id(),
            &invocation("invocation-1"),
            INSIDE,
            Some(LifetimeFailpoint::AfterDurableAdmission),
        )
        .err();
    assert_eq!(
        admitted,
        Some(ContextLifetimeError::InterruptedAfterAdmission)
    );
    let held = ledger.manifests()[0].clone();

    let settled = ledger
        .reconcile("invocation-1", DispatchSettlement::Dispatched)
        .expect("reconciliation succeeds");
    assert_eq!(settled.ordinal, held.ordinal);
    assert_eq!(settled.invocation_id, held.invocation_id);
    assert_eq!(settled.items, held.items);
    assert_eq!(settled.admitted_at, held.admitted_at);
    assert_eq!(settled.manifest_digest, held.manifest_digest);
    settled
        .verify()
        .expect("the settled record still binds its bytes");
}

/// AC3: a forged manifest whose carried bytes disagree with its fields is refused, so a record
/// cannot look intact while carrying a different item list.
#[test]
fn a_forged_manifest_is_refused() {
    let mut ledger = ledger_with(next_n_scope(2));
    let mut manifest = ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    manifest.bytes = b"{\"forged\":true}".to_vec();
    assert_eq!(manifest.verify(), Err(ContextLifetimeError::InvalidInput));

    let mut other = ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    other.manifest_digest = "0".repeat(64);
    assert_eq!(other.verify(), Err(ContextLifetimeError::InvalidInput));
}
