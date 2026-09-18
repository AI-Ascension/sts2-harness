// SPDX-License-Identifier: MIT

//! AC4 — sibling agents, branches, episodes and runs cannot inherit scope, and old manifests stay
//! immutable and inspectable under retention policy (issue #111).
//!
//! Synthetic fixtures only; no provider, host, game, or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_lifetime.rs"]
mod fixture;

use fixture::{
    CEILING, INSIDE, branch_free_scope, invocation, invocation_by, ledger_with, next_n_scope,
    owner, scope_id, sibling_agent_owner, sibling_branch_owner, sibling_episode_owner,
    sibling_run_owner,
};
use sts2_harness::context_control::{
    CONTEXT_LIFETIME_SCHEMA, ContextLifetimeError, ContextLifetimeScope,
};

/// AC4: a scope pinned to one branch refuses the same agent on a sibling branch.
#[test]
fn sibling_branch_cannot_consume_a_branch_pinned_scope() {
    let mut ledger = ledger_with(next_n_scope(3));
    let refused = ledger.admit(
        scope_id(),
        &invocation_by(sibling_branch_owner(), "invocation-sibling"),
        INSIDE,
        None,
    );
    assert_eq!(
        refused,
        Err(ContextLifetimeError::SiblingScopeRefused {
            scope_id: scope_id().to_owned(),
        })
    );
}

/// AC4: sibling agent, episode and run scopes are all refused, so the guarantee is owner-wide and
/// not merely a branch check.
#[test]
fn every_sibling_owner_axis_is_refused() {
    for sibling in [
        sibling_agent_owner(),
        sibling_episode_owner(),
        sibling_run_owner(),
        sibling_branch_owner(),
    ] {
        let mut ledger = ledger_with(next_n_scope(3));
        let refused = ledger.admit(
            scope_id(),
            &invocation_by(sibling, "invocation-sibling"),
            INSIDE,
            None,
        );
        assert!(
            matches!(
                refused,
                Err(ContextLifetimeError::SiblingScopeRefused { .. })
            ),
            "a sibling owner must be refused, got {refused:?}"
        );
    }
}

/// AC4: a refused sibling consumes nothing — no capacity, no manifest, no held slot — so the
/// authorized owner still sees the full declared window.
#[test]
fn sibling_refusal_consumes_nothing() {
    let mut ledger = ledger_with(next_n_scope(3));
    for _ in 0..4 {
        let _ = ledger.admit(
            scope_id(),
            &invocation_by(sibling_agent_owner(), "invocation-sibling"),
            INSIDE,
            None,
        );
    }
    assert!(
        ledger.manifests().is_empty(),
        "no sibling manifest is recorded"
    );
    let preview = ledger
        .preview(scope_id(), INSIDE)
        .expect("preview succeeds");
    assert_eq!(preview.consumed, 0);
    assert_eq!(preview.remaining, 3);
    assert!(preview.held.is_empty());

    // The authorized owner is unaffected and still consumes ordinal 1.
    let manifest = ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("the authorized owner admits");
    assert_eq!(manifest.ordinal, 1);
}

/// AC4: a scope issued without a branch identity authorizes the owning agent in any branch, which
/// is the documented live seam where no branch identity is available.
#[test]
fn branch_free_scope_authorizes_any_branch() {
    let mut ledger = ledger_with(branch_free_scope());
    let admitted = ledger
        .admit(
            scope_id(),
            &invocation_by(sibling_branch_owner(), "invocation-other-branch"),
            INSIDE,
            None,
        )
        .expect("a branch-free scope authorizes the agent in any branch");
    assert_eq!(admitted.ordinal, 1);
    assert_eq!(admitted.owner.branch_id.as_deref(), Some("branch-2"));
}

/// AC4: a branch-free scope still refuses a different agent, so relaxing the branch axis does not
/// relax the agent axis.
#[test]
fn branch_free_scope_still_refuses_a_sibling_agent() {
    let mut ledger = ledger_with(branch_free_scope());
    let refused = ledger.admit(
        scope_id(),
        &invocation_by(sibling_agent_owner(), "invocation-sibling"),
        INSIDE,
        None,
    );
    assert!(matches!(
        refused,
        Err(ContextLifetimeError::SiblingScopeRefused { .. })
    ));
}

/// AC4: expiry never deletes or rewrites history. The manifests admitted before the ceiling remain
/// inspectable, byte-for-byte, and still verify against their own carried bytes.
#[test]
fn old_manifests_stay_inspectable_after_expiry() {
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let admitted = ledger.manifests().to_vec();
    assert_eq!(admitted.len(), 1);

    let after_ceiling = CEILING + 1;
    let expired = ledger
        .preview(scope_id(), after_ceiling)
        .expect("preview succeeds");
    assert!(expired.expired, "the ceiling has passed");
    assert_eq!(
        expired.consumed, 1,
        "expiry does not erase the record of what was admitted"
    );

    assert_eq!(
        ledger.manifests(),
        admitted.as_slice(),
        "expiry must not rewrite or drop admitted manifests"
    );
    for manifest in ledger.manifests() {
        manifest
            .verify()
            .expect("an expired manifest still binds its canonical bytes");
    }
}

/// AC4: past the ceiling the scope is refused and cannot resurrect applicability, while the
/// historical manifests remain readable under retention policy.
#[test]
fn expiry_refuses_admission_without_rewriting_history() {
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let before = ledger.manifests().to_vec();

    let refused = ledger.admit(scope_id(), &invocation("invocation-2"), CEILING, None);
    assert_eq!(
        refused,
        Err(ContextLifetimeError::Expired {
            scope_id: scope_id().to_owned(),
        })
    );
    assert_eq!(ledger.manifests(), before.as_slice());
    let preview = ledger
        .preview(scope_id(), CEILING)
        .expect("preview succeeds");
    assert_eq!(
        preview.consumed, 1,
        "the refused admission consumed nothing"
    );
}

/// AC4: an approval minted inside the window verifies at the same revision, then fails once the
/// ceiling passes — expiry is evaluated at verification time rather than trusted from mint time.
#[test]
fn approval_verifies_then_fails_after_the_ceiling() {
    let mut ledger = ledger_with(next_n_scope(2));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let approval = ledger.approve(scope_id()).expect("approval mints");
    let manifests = ledger.manifests().to_vec();

    approval
        .verify(scope_id(), &approval.scope_digest, INSIDE, &manifests)
        .expect("the approval holds inside the window");

    let after_ceiling = CEILING + 1;
    assert_eq!(
        approval.verify(
            scope_id(),
            &approval.scope_digest,
            after_ceiling,
            &manifests
        ),
        Err(ContextLifetimeError::Expired {
            scope_id: scope_id().to_owned(),
        }),
        "a preview obtained before the ceiling cannot be replayed after it"
    );
}

/// AC4: an approval is bound to the exact manifest set it was minted over, so a later admission
/// invalidates it instead of being silently absorbed.
#[test]
fn approval_is_invalidated_when_the_manifest_set_changes() {
    let mut ledger = ledger_with(next_n_scope(3));
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    let approval = ledger.approve(scope_id()).expect("approval mints");

    ledger
        .admit(scope_id(), &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");
    let grown = ledger.manifests().to_vec();
    assert_eq!(grown.len(), 2);
    assert_eq!(
        approval.verify(scope_id(), &approval.scope_digest, INSIDE, &grown),
        Err(ContextLifetimeError::InvalidInput)
    );
}

/// AC4: manifests are filed under the scope that admitted them, so one scope's history cannot be
/// presented as another's.
#[test]
fn manifests_are_filed_under_their_own_scope() {
    let second = ContextLifetimeScope {
        schema: CONTEXT_LIFETIME_SCHEMA.to_owned(),
        scope_id: "scope-2".to_owned(),
        owner: owner(),
        ..next_n_scope(2)
    };
    let mut ledger = ledger_with(next_n_scope(2));
    ledger.issue(second).expect("a distinct scope issues");
    ledger
        .admit(scope_id(), &invocation("invocation-1"), INSIDE, None)
        .expect("admission succeeds");
    ledger
        .admit("scope-2", &invocation("invocation-2"), INSIDE, None)
        .expect("admission succeeds");

    assert_eq!(ledger.manifests_for(scope_id()).len(), 1);
    assert_eq!(ledger.manifests_for(scope_id())[0].scope_id, scope_id());
    assert_eq!(ledger.manifests_for("scope-2").len(), 1);
    assert_eq!(
        ledger.manifests_for("scope-2")[0].invocation_id,
        "invocation-2"
    );
    assert_eq!(ledger.manifests().len(), 2, "history is never dropped");
}

/// AC4: a scope identifier reused for a different owner is refused, so a caller cannot rebind an
/// existing scope to itself and inherit its remaining window.
#[test]
fn a_scope_id_cannot_be_rebound_to_a_sibling_owner() {
    let mut ledger = ledger_with(next_n_scope(3));
    let rebound = ContextLifetimeScope {
        owner: sibling_agent_owner(),
        ..next_n_scope(3)
    };
    assert_eq!(
        ledger.issue(rebound),
        Err(ContextLifetimeError::RepeatedScope {
            scope_id: scope_id().to_owned(),
        })
    );
}
