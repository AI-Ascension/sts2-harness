// SPDX-License-Identifier: MIT

//! Per-invocation membership: scope, three-invocation equality, and pre-dispatch gates (issue #106).
//!
//! Synthetic fixtures only; no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

#[path = "support/context_control_membership.rs"]
mod fixture;

use fixture::{FUTURE, NOW, check, item, policy, registry_history, scope, visible_ids};
use sts2_harness::context_control::{
    ContextDraft, ContextMembershipError, MAX_CONTEXT_ITEMS, MembershipDisposition,
    prevalidate_and_bind, resolve_membership,
};

// 1. Include / exclude / include across three logical invocations yields the same effective bytes
//    as invocation 1, while overall collection continues.
#[test]
fn three_invocations_converge_and_collection_continues() {
    let (registry, references) = registry_history(&["alpha", "beta", "gamma"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = references.clone();

    // Invocation 1: include everything.
    let first = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    let first_effective =
        resolve_membership(&first, &draft, &registry, &scope(), NOW).expect("invocation 1");
    let first_bytes: Vec<Vec<u8>> = first_effective
        .model_visible
        .iter()
        .map(|reference| {
            registry
                .get(&format!("{}:{}", reference.item_id, reference.version))
                .expect("registered")
                .bytes
                .clone()
        })
        .collect();
    assert_eq!(
        first_bytes,
        vec![b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()],
        "invocation 1 must carry exactly the collected history bytes"
    );

    // Invocation 2: exclude the middle item.
    let second = policy(
        "invocation-2",
        MembershipDisposition::Exclude,
        vec![references[1].clone()],
    );
    let second_effective =
        resolve_membership(&second, &draft, &registry, &scope(), NOW).expect("invocation 2");
    assert_eq!(
        visible_ids(&second_effective),
        vec!["history-0".to_owned(), "history-2".to_owned()],
        "invocation 2 must exclude only the named item"
    );

    // Invocation 3: include again.
    let third = policy("invocation-3", MembershipDisposition::Include, Vec::new());
    let third_effective =
        resolve_membership(&third, &draft, &registry, &scope(), NOW).expect("invocation 3");
    let third_bytes: Vec<Vec<u8>> = third_effective
        .model_visible
        .iter()
        .map(|reference| {
            registry
                .get(&format!("{}:{}", reference.item_id, reference.version))
                .expect("registered")
                .bytes
                .clone()
        })
        .collect();
    assert_eq!(
        third_bytes,
        vec![b"alpha".to_vec(), b"beta".to_vec(), b"gamma".to_vec()],
        "invocation 3 must reproduce the exact expected application bytes"
    );
    assert_eq!(
        first_bytes, third_bytes,
        "invocation 3 must reproduce invocation 1's exact model-visible bytes"
    );
    assert_eq!(visible_ids(&first_effective), visible_ids(&third_effective));

    // Collection continues: the registry still holds every item, including the excluded one, and
    // exclusion orchestration never deleted it.
    assert_eq!(
        registry.len(),
        3,
        "collection must not be narrowed by a policy"
    );
    assert_eq!(second_effective.excluded, vec![references[1].clone()]);
}

// 2. Node defaults/overrides and pins cannot leak to a sibling agent/branch/episode; an explicit
//    authorized wider scope is admitted and revalidated.
#[test]
fn sibling_scope_items_are_refused_unless_explicitly_authorized() {
    let mut registry = BTreeMap::new();
    // A shared, carryable item.
    let shared = item("history-1", 1, "history", "shared", false);
    registry.insert("history-1:1".to_owned(), shared.clone());
    // An invocation-scoped item: any kind outside the shared list.
    let scoped = item("episode-9-note", 1, "episode_note", "sibling", false);
    registry.insert("episode-9-note:1".to_owned(), scoped.clone());
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = vec![shared.reference.clone(), scoped.reference.clone()];

    // Without authorization the scoped item is refused as a sibling leak.
    let unauthorized = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    assert_eq!(
        resolve_membership(&unauthorized, &draft, &registry, &scope(), NOW),
        Err(ContextMembershipError::SiblingScopeLeak {
            item_id: "episode-9-note".to_owned()
        }),
        "an invocation-scoped item must not leak without an explicit wider scope"
    );

    // Authorization that names this agent but not the item grants the capability without covering
    // the specific item, so it is refused precisely rather than as a plain sibling leak.
    let mut partial = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    partial.broader_scope.authorized_agent_ids = vec!["agent-1".to_owned()];
    assert_eq!(
        resolve_membership(&partial, &draft, &registry, &scope(), NOW),
        Err(ContextMembershipError::WiderScopeNotAuthorized {
            item_id: "episode-9-note".to_owned()
        })
    );

    // Naming the item without authorizing this agent is refused as a sibling leak: this caller was
    // never granted the wider scope, even though another agent was.
    let mut wrong_agent = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    wrong_agent.broader_scope.items = vec![scoped.reference.clone()];
    wrong_agent.broader_scope.authorized_agent_ids = vec!["agent-2".to_owned()];
    assert_eq!(
        resolve_membership(&wrong_agent, &draft, &registry, &scope(), NOW),
        Err(ContextMembershipError::SiblingScopeLeak {
            item_id: "episode-9-note".to_owned()
        })
    );

    // Complete authorization admits it, and revalidation keeps admitting it.
    let mut authorized = policy("invocation-1", MembershipDisposition::Include, Vec::new());
    authorized.broader_scope.items = vec![scoped.reference.clone()];
    authorized.broader_scope.authorized_agent_ids = vec!["agent-1".to_owned()];
    let prepared = prevalidate_and_bind(
        &authorized,
        &draft,
        &registry,
        NOW,
        &check(),
        MAX_CONTEXT_ITEMS,
    )
    .expect("authorized wider scope");
    assert_eq!(
        prepared
            .dispatch_view
            .included
            .iter()
            .map(|reference| reference.item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["episode-9-note", "history-1"]
    );
    prepared
        .effective
        .revalidate(
            &authorized,
            &draft,
            &registry,
            &check(),
            NOW,
            MAX_CONTEXT_ITEMS,
        )
        .expect("revalidated wider scope");

    // Dropping the item from the authorization stops the next revalidation, while the agent
    // capability itself is still held.
    let mut revoked = authorized.clone();
    revoked.broader_scope.items.clear();
    assert_eq!(
        prepared.effective.revalidate(
            &revoked,
            &draft,
            &registry,
            &check(),
            NOW,
            MAX_CONTEXT_ITEMS
        ),
        Err(ContextMembershipError::WiderScopeNotAuthorized {
            item_id: "episode-9-note".to_owned()
        })
    );
}

// 3. Revoked/expired pins, protected exclusions, and mandatory-plus-pin overflow fail before
//    dispatch with precise reasons and no dispatch view.
#[test]
fn pre_dispatch_gates_fail_closed_with_precise_reasons() {
    let (registry, references) = registry_history(&["alpha", "beta"]);
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    draft.selected_items = references.clone();
    draft.pinned_item_ids = vec!["history-0".to_owned()];
    let include = policy("invocation-1", MembershipDisposition::Include, Vec::new());

    // A revoked item id inside the effective set fails before dispatch.
    let mut revoked = check();
    revoked.revoked_item_ids = vec!["history-1".to_owned()];
    assert_eq!(
        prevalidate_and_bind(
            &include,
            &draft,
            &registry,
            NOW,
            &revoked,
            MAX_CONTEXT_ITEMS
        ),
        Err(ContextMembershipError::RevokedOrExpired {
            subject: "history-1".to_owned()
        })
    );

    // A revoked invocation fails before dispatch.
    let mut revoked_invocation = check();
    revoked_invocation.revoked_invocation_ids = vec!["invocation-1".to_owned()];
    assert_eq!(
        prevalidate_and_bind(
            &include,
            &draft,
            &registry,
            NOW,
            &revoked_invocation,
            MAX_CONTEXT_ITEMS
        ),
        Err(ContextMembershipError::RevokedOrExpired {
            subject: "invocation-1".to_owned()
        })
    );

    // An expired item fails at preparation.
    let mut expired_registry = registry.clone();
    let mut expired = expired_registry
        .get("history-0:1")
        .expect("registered")
        .clone();
    expired.expires_at = NOW;
    expired_registry.insert("history-0:1".to_owned(), expired);
    assert_eq!(
        prevalidate_and_bind(
            &include,
            &draft,
            &expired_registry,
            NOW,
            &check(),
            MAX_CONTEXT_ITEMS
        ),
        Err(ContextMembershipError::RevokedOrExpired {
            subject: "history-0".to_owned()
        })
    );

    // Excluding an owner prerequisite is refused; it may never be dropped from owner state.
    let mut protected_registry = registry.clone();
    let mut protected_item = protected_registry
        .get("history-1:1")
        .expect("registered")
        .clone();
    protected_item.protected = true;
    protected_registry.insert("history-1:1".to_owned(), protected_item);
    let exclude = policy(
        "invocation-1",
        MembershipDisposition::Exclude,
        vec![references[1].clone()],
    );
    assert_eq!(
        prevalidate_and_bind(
            &exclude,
            &draft,
            &protected_registry,
            NOW,
            &check(),
            MAX_CONTEXT_ITEMS
        ),
        Err(ContextMembershipError::ProtectedPrerequisiteExcluded {
            item_id: "history-1".to_owned()
        })
    );

    // A protected included item is retained but suppressed from model-visible input.
    let mut protected_included = registry.clone();
    let mut prerequisite = item("history-1", 1, "history", "beta", true);
    prerequisite.expires_at = FUTURE;
    protected_included.insert("history-1:1".to_owned(), prerequisite);
    let prepared = prevalidate_and_bind(
        &include,
        &draft,
        &protected_included,
        NOW,
        &check(),
        MAX_CONTEXT_ITEMS,
    )
    .expect("protected prerequisite is retained, not rejected");
    assert_eq!(
        prepared
            .effective
            .mandatory
            .iter()
            .map(|reference| reference.item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["history-1"]
    );
    assert_eq!(
        prepared
            .dispatch_view
            .model_visible
            .iter()
            .map(|reference| reference.item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["history-0"],
        "a prerequisite must stay out of model-visible input"
    );

    // A plain draft selection that exceeds the bound is reported as an ordinary bound overflow,
    // while a non-negotiable prerequisite overflow is reported precisely as mandatory/pin.
    assert_eq!(
        prevalidate_and_bind(&include, &draft, &registry, NOW, &check(), 1),
        Err(ContextMembershipError::TooManyItems { bound: 1 })
    );

    // A protected prerequisite that alone exceeds the bound cannot be resolved by any selector.
    let mut over_bound = registry.clone();
    let mut mandatory_item = over_bound.get("history-0:1").expect("registered").clone();
    mandatory_item.protected = true;
    over_bound.insert("history-0:1".to_owned(), mandatory_item);
    let mut mandatory_draft = draft.clone();
    mandatory_draft.selected_items = vec![references[0].clone()];
    assert_eq!(
        prevalidate_and_bind(&include, &mandatory_draft, &over_bound, NOW, &check(), 0),
        Err(ContextMembershipError::MandatoryPinOverflow { bound: 0 })
    );

    // A policy-added item that pushes past the bound is attributed to the mandatory/pin rule.
    let mut added_draft = draft.clone();
    added_draft.selected_items = vec![references[0].clone()];
    let added = policy(
        "invocation-1",
        MembershipDisposition::Include,
        vec![references[1].clone()],
    );
    assert_eq!(
        prevalidate_and_bind(&added, &added_draft, &registry, NOW, &check(), 1),
        Err(ContextMembershipError::MandatoryPinOverflow { bound: 1 })
    );
}

// 4. An unpin affects only future input: the historical digest is unchanged, the next preparation
//    differs.
