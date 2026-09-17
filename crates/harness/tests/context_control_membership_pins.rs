// SPDX-License-Identifier: MIT

//! Per-invocation membership: pin revalidation, digests, and effective absence (issue #106).
//!
//! Synthetic fixtures only; no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/context_control_membership.rs"]
mod fixture;

use fixture::{NOW, check, policy, registry_history, scope, visible_ids};
use sts2_harness::context_control::{
    ContextDraft, ContextMembershipError, ContextMembershipScope, ContextModelView,
    MAX_CONTEXT_ITEMS, MembershipContinuity, MembershipDisposition, MembershipReasonCode,
    prevalidate_and_bind, resolve_membership,
};

#[test]
fn unpin_affects_only_future_input() {
    let (registry, references) = registry_history(&["alpha", "beta"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = references.clone();
    draft.pinned_item_ids = vec!["history-0".to_owned(), "history-1".to_owned()];

    let mut inherit = policy("invocation-1", MembershipDisposition::Inherit, Vec::new());
    inherit.inherit_pins = true;
    let historical = prevalidate_and_bind(
        &inherit,
        &draft,
        &registry,
        NOW,
        &check(),
        MAX_CONTEXT_ITEMS,
    )
    .expect("historical preparation");
    let historical_digest = historical.policy_digest.clone();
    let historical_pins = historical.effective.pins.clone();
    assert_eq!(
        historical_pins,
        vec!["history-0".to_owned(), "history-1".to_owned()]
    );

    // The operator unpins one item.
    let mut unpinned_draft = draft.clone();
    unpinned_draft.pinned_item_ids = vec!["history-0".to_owned()];
    // Re-preparing the untouched draft must reproduce the historical digest and pins exactly.
    let replayed = prevalidate_and_bind(
        &inherit,
        &draft,
        &registry,
        NOW,
        &check(),
        MAX_CONTEXT_ITEMS,
    )
    .expect("replayed preparation");
    assert_eq!(
        replayed.policy_digest, historical_digest,
        "historical evidence must stay reproducible from the same input"
    );
    assert_eq!(
        replayed.effective.pins, historical_pins,
        "an unpin must not rewrite retained evidence"
    );

    let future = prevalidate_and_bind(
        &inherit,
        &unpinned_draft,
        &registry,
        NOW,
        &check(),
        MAX_CONTEXT_ITEMS,
    )
    .expect("future preparation");
    assert_eq!(future.effective.pins, vec!["history-0".to_owned()]);
    assert_ne!(
        future.effective.pins, historical_pins,
        "the unpin must change only the future input"
    );

    // An excluded item cannot stay pinned in an `Inherit` policy either.
    let mut excluded_draft = draft.clone();
    excluded_draft.selected_items = vec![references[0].clone()];
    let narrowed = prevalidate_and_bind(
        &inherit,
        &excluded_draft,
        &registry,
        NOW,
        &check(),
        MAX_CONTEXT_ITEMS,
    )
    .expect("narrowed preparation");
    assert_eq!(narrowed.effective.pins, vec!["history-0".to_owned()]);
    assert!(!narrowed.effective.pins.contains(&"history-1".to_owned()));
}

// 5. Effective absence of the observation is refused for a continuity that cannot execute it.
#[test]
fn opaque_history_rejects_effective_absence() {
    let (registry, references) = registry_history(&["alpha"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = references;
    let mut hidden = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    hidden.model_view = ContextModelView {
        observation_visible: false,
    };

    let mut opaque = check();
    opaque.continuity = MembershipContinuity::OpaquePersistent;
    assert_eq!(
        prevalidate_and_bind(&hidden, &draft, &registry, NOW, &opaque, MAX_CONTEXT_ITEMS),
        Err(ContextMembershipError::EffectiveAbsenceUnsupported),
        "a selector cannot erase provider history"
    );

    // The same policy is admissible only when continuity can actually execute it.
    let prepared =
        prevalidate_and_bind(&hidden, &draft, &registry, NOW, &check(), MAX_CONTEXT_ITEMS)
            .expect("stateless continuity can execute effective absence");
    assert!(!prepared.dispatch_view.observation_visible);
}

// 6. The policy digest is stable for one policy and changes for any field.
#[test]
fn policy_digest_is_stable_and_discriminating() {
    let base = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    let digest = base.digest().expect("digest");
    assert_eq!(
        digest,
        base.digest().expect("stable"),
        "digest must be stable"
    );
    assert_eq!(digest.len(), 64);

    let mut changed = base.clone();
    changed.invocation_id = "invocation-2".to_owned();
    assert_ne!(digest, changed.digest().expect("digest"));

    let mut changed_disposition = base.clone();
    changed_disposition.disposition = MembershipDisposition::Exclude;
    assert_ne!(digest, changed_disposition.digest().expect("digest"));

    let mut changed_view = base.clone();
    changed_view.model_view = ContextModelView {
        observation_visible: false,
    };
    assert_ne!(digest, changed_view.digest().expect("digest"));

    // A policy that changed after preparation is refused at revalidation.
    let (registry, references) = registry_history(&["alpha"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = references;
    let prepared = prevalidate_and_bind(&base, &draft, &registry, NOW, &check(), MAX_CONTEXT_ITEMS)
        .expect("prepared");
    let mut moved = base.clone();
    moved.model_view = ContextModelView {
        observation_visible: false,
    };
    assert_eq!(
        prepared
            .effective
            .revalidate(&moved, &draft, &registry, &check(), NOW, MAX_CONTEXT_ITEMS),
        Err(ContextMembershipError::PolicyChanged)
    );
}

#[test]
fn invalid_policies_and_scopes_are_refused() {
    let (registry, references) = registry_history(&["alpha"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = references;

    let mut wrong_schema = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    wrong_schema.schema = "ascension.context-control.membership.v2".to_owned();
    assert!(matches!(
        resolve_membership(&wrong_schema, &draft, &registry, &scope(), NOW),
        Err(ContextMembershipError::InvalidInput(_))
    ));

    // `Inherit` may not widen anything.
    let mut inherit_with_overrides = policy(
        "invocation-1",
        MembershipDisposition::Inherit,
        vec![draft.selected_items[0].clone()],
    );
    inherit_with_overrides.inherit_pins = true;
    assert!(!inherit_with_overrides.valid());

    // A policy from a different draft revision is refused.
    let mut wrong_revision = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    wrong_revision.base_revision_id = "revision-9".to_owned();
    assert!(matches!(
        resolve_membership(&wrong_revision, &draft, &registry, &scope(), NOW),
        Err(ContextMembershipError::InvalidInput(_))
    ));

    // A malformed caller scope is refused.
    let invalid_scope = ContextMembershipScope {
        run_id: String::new(),
        ..scope()
    };
    assert!(matches!(
        resolve_membership(
            &policy("invocation-1", MembershipDisposition::Include, Vec::new()),
            &draft,
            &registry,
            &invalid_scope,
            NOW
        ),
        Err(ContextMembershipError::InvalidInput(_))
    ));

    // An excluded item that is not part of the draft selection is refused.
    let (wide_registry, wide_references) = registry_history(&["alpha", "beta"]);
    let mut wide_draft = ContextDraft::new("draft-1", "revision-1");
    wide_draft.selected_items = vec![wide_references[0].clone()];
    let exclude_unselected = policy(
        "invocation-1",
        MembershipDisposition::Exclude,
        vec![wide_references[1].clone()],
    );
    assert!(matches!(
        resolve_membership(
            &exclude_unselected,
            &wide_draft,
            &wide_registry,
            &scope(),
            NOW
        ),
        Err(ContextMembershipError::InvalidInput(_))
    ));
}

#[test]
fn duplicate_overrides_and_unselected_items_are_ordered_and_explained() {
    let (registry, references) = registry_history(&["alpha", "beta"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = vec![references[0].clone()];

    // Naming the same reference twice is refused rather than silently collapsed, so an ambiguous
    // override set never resolves.
    let mut duplicate = policy(
        "invocation-1",
        MembershipDisposition::Include,
        vec![references[0].clone(), references[0].clone()],
    );
    assert!(!duplicate.valid(), "duplicate overrides are invalid");
    duplicate.overrides = vec![references[1].clone()];
    let effective =
        resolve_membership(&duplicate, &draft, &registry, &scope(), NOW).expect("resolved");
    assert_eq!(
        visible_ids(&effective),
        vec!["history-0".to_owned(), "history-1".to_owned()]
    );
    // The unselected registry item is still explained, so the effective set is fully accounted for.
    let explained = effective
        .decisions
        .iter()
        .filter(|decision| decision.reason == MembershipReasonCode::NotSelected)
        .count();
    assert_eq!(explained, 0, "both items are selected here");

    // With no overrides, the unselected item is reported as not selected.
    let plain = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    let plain_effective =
        resolve_membership(&plain, &draft, &registry, &scope(), NOW).expect("resolved");
    assert!(
        plain_effective
            .decisions
            .iter()
            .any(
                |decision| decision.reason == MembershipReasonCode::NotSelected
                    && decision.reference.item_id == "history-1"
            )
    );
    assert!(
        plain_effective
            .decisions
            .iter()
            .any(
                |decision| decision.reason == MembershipReasonCode::SelectedByDraft
                    && decision.reference.item_id == "history-0"
            )
    );
}
