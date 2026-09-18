// SPDX-License-Identifier: MIT

//! AC1 — one-invocation and next-N applicability reach exactly the admitted logical invocations,
//! with injected clock boundaries (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use fixture::{
    CEILING, INSIDE, ISSUED_AT, current_invocation_scope, invocation, items, ledger_with,
    next_n_scope, owner, scope_id,
};
use sts2_harness::context_control::{ContextLifetimeError, LifetimeApplicability};

/// AC1: a one-invocation scope admits exactly one distinct logical invocation.
#[test]
fn current_invocation_scope_admits_exactly_one() {
    let mut ledger = ledger_with(current_invocation_scope());
    let first = ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("first admission succeeds");
    assert_eq!(
        first.applicability,
        LifetimeApplicability::CurrentInvocation
    );
    assert_eq!(first.ordinal, 1);
    assert_eq!(first.remaining_after, 0);

    let refused = ledger.admit(scope_id(), &invocation("invocation-2"), INSIDE, None);
    assert_eq!(
        refused,
        Err(ContextLifetimeError::Exhausted {
            scope_id: scope_id().to_owned()
        })
    );
    assert_eq!(ledger.manifests().len(), 1);
}

/// AC1: a next-N scope admits the current invocation plus the next N-1 distinct ones, in order.
#[test]
fn next_n_scope_admits_exactly_the_declared_window() {
    let mut ledger = ledger_with(next_n_scope(3));
    let mut remaining = Vec::new();
    for index in 1..=3 {
        let manifest = ledger
            .admit(
                scope_id(),
                &invocation(&format!("invocation-{index}")),
                INSIDE,
                None,
            )
            .expect("admission inside the window succeeds");
        assert_eq!(manifest.ordinal, index);
        remaining.push(manifest.remaining_after);
    }
    assert_eq!(remaining, vec![2, 1, 0]);
    assert_eq!(ledger.manifests().len(), 3);

    let refused = ledger.admit(scope_id(), &invocation("invocation-4"), INSIDE, None);
    assert!(matches!(
        refused,
        Err(ContextLifetimeError::Exhausted { .. })
    ));
}

/// AC1: the manifest carries the scope's items, owner and scope digest, so a consumer can prove
/// *why* the invocation was inside the window instead of trusting a counter.
#[test]
fn admitted_manifest_records_its_scope_provenance() {
    let mut ledger = ledger_with(next_n_scope(2));
    let manifest = ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    assert_eq!(manifest.items, items());
    assert_eq!(manifest.owner, owner());
    assert_eq!(manifest.scope_id, scope_id());
    assert_eq!(manifest.admitted_at, INSIDE);
    assert_eq!(manifest.scope_digest.len(), 64);
    assert_eq!(manifest.manifest_digest.len(), 64);
    manifest.verify().expect("manifest binds its own bytes");
}

/// AC1: injected clock boundaries decide admission. The instant before the ceiling is admitted; the
/// ceiling itself and anything later is refused, so the boundary is inclusive-exclusive and exact.
#[test]
fn injected_clock_ceiling_bounds_admission_exactly() {
    let mut ledger = ledger_with(next_n_scope(4));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), CEILING - 1, None)
        .expect("the instant before the ceiling is inside the window");

    let at_ceiling = ledger.admit(scope_id(), &invocation("invocation-2"), CEILING, None);
    assert_eq!(
        at_ceiling,
        Err(ContextLifetimeError::Expired {
            scope_id: scope_id().to_owned()
        })
    );
    let after_ceiling = ledger.admit(scope_id(), &invocation("invocation-3"), CEILING + 1, None);
    assert!(matches!(
        after_ceiling,
        Err(ContextLifetimeError::Expired { .. })
    ));
    assert_eq!(
        ledger.manifests().len(),
        1,
        "a refused admission consumes nothing"
    );
}

/// AC1: issuance itself is validated, so a malformed or already-spent window never exists.
#[test]
fn issuance_refuses_a_degenerate_window() {
    let mut ledger = ledger_with(next_n_scope(2));
    assert_eq!(
        ledger.issue(next_n_scope(0)),
        Err(ContextLifetimeError::EmptyBound)
    );

    let inverted = sts2_harness::context_control::ContextLifetimeScope {
        issued_at: CEILING,
        ceiling: ISSUED_AT,
        ..current_invocation_scope()
    };
    assert_eq!(
        ledger.issue(inverted),
        Err(ContextLifetimeError::InvertedCeiling)
    );

    let no_items = sts2_harness::context_control::ContextLifetimeScope {
        items: Vec::new(),
        ..current_invocation_scope()
    };
    assert_eq!(
        ledger.issue(no_items),
        Err(ContextLifetimeError::InvalidInput)
    );
}

/// AC1: a scope reused under the same id must describe the same window, so an edited declaration
/// cannot silently inherit the original's verdict.
#[test]
fn a_reused_scope_id_must_describe_the_same_window() {
    let mut ledger = ledger_with(next_n_scope(2));
    let mut widened = next_n_scope(8);
    widened.scope_id = scope_id().to_owned();
    assert_eq!(
        ledger.issue(widened),
        Err(ContextLifetimeError::RepeatedScope {
            scope_id: scope_id().to_owned()
        })
    );
    assert_eq!(
        ledger.issue(next_n_scope(2)).expect("identical re-issue"),
        ledger
            .preview(scope_id(), INSIDE)
            .expect("preview succeeds")
            .scope_digest
    );
}
