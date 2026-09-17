// SPDX-License-Identifier: MIT

//! End-to-end membership enforcement through the served managed render path (issue #106).
//!
//! These tests drive [`ProductionLiveWorkflowSession::decide_for`] — the live dispatch seam — with
//! an owner-issued selector in force, so they fail if the membership boundary is unwired from the
//! render path rather than merely absent from it. Synthetic transport only; no provider is
//! contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, dead_code)]

use super::managed_render_tests::{render_fixture, render_test_session, selected_limits};
use super::*;
use crate::context_control::{
    ContextMembershipSelector, ContextModelView, MembershipContinuity, MembershipDisposition,
};
use crate::{
    ContextDraft, ContextItem, ContextItemRef, ContextRenderLimits, ContextRenderSource,
    ContextSourceDocument, ExoConfig,
};
use std::collections::BTreeMap;

/// Registry and draft holding three shared, unprotected history items.
fn history_document() -> (ContextSourceDocument, Vec<ContextItemRef>) {
    let mut items = BTreeMap::new();
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    let mut references = Vec::new();
    for (item_id, content) in [
        ("history-0", &b"alpha"[..]),
        ("history-1", &b"beta"[..]),
        ("history-2", &b"gamma"[..]),
    ] {
        let item = ContextItem {
            reference: ContextItemRef {
                item_id: item_id.to_owned(),
                version: 1,
                sha256: crate::sha256_hex(content),
            },
            kind: "history".to_owned(),
            bytes: content.to_vec(),
            protected: false,
            expires_at: 100,
        };
        draft.selected_items.push(item.reference.clone());
        references.push(item.reference.clone());
        items.insert(format!("{item_id}:1"), item);
    }
    (ContextSourceDocument { draft, items }, references)
}

/// A render source bound to one invocation, optionally under a membership selector.
fn membership_source(
    document: ContextSourceDocument,
    invocation_id: &str,
    selector: Option<ContextMembershipSelector>,
) -> (ContextRenderSource, ExoConfig) {
    let (mut source, config) = render_fixture();
    source.document = document;
    source.identity.invocation_id = invocation_id.to_owned();
    source.identity.membership_digest = selector
        .as_ref()
        .and_then(|selector| selector.digest().ok());
    source.membership = selector;
    source.continuity = MembershipContinuity::Stateless;
    (source, config)
}

/// Runs one invocation and returns its exact provider bytes and exchange count.
fn serve(source: ContextRenderSource, config: ExoConfig) -> (Vec<Vec<u8>>, usize) {
    let (mut session, _, exchanges, requests) = render_test_session(
        source,
        config,
        ContextRenderLimits::harness_maxima(),
        Default::default(),
        None,
    );
    session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("served managed decision");
    let recorded = requests.lock().expect("recorded requests").clone();
    (recorded, exchanges.load(Ordering::SeqCst))
}

fn contains(requests: &[Vec<u8>], needle: &[u8]) -> bool {
    requests
        .iter()
        .any(|request| request.windows(needle.len()).any(|window| window == needle))
}

// AC1: include / exclude / include across three invocations reproduces the exact bytes of the
// first invocation, while collection continues to hold the excluded item.
#[test]
fn three_invocations_reproduce_exact_bytes_through_the_render_path() {
    let (document, references) = history_document();
    let include = ContextMembershipSelector::include();
    let excluding = ContextMembershipSelector {
        disposition: MembershipDisposition::Exclude,
        overrides: vec![references[1].clone()],
        ..ContextMembershipSelector::include()
    };

    let (first, config) =
        membership_source(document.clone(), "invocation-1", Some(include.clone()));
    let (first_requests, first_exchanges) = serve(first, config);
    let (second, config) =
        membership_source(document.clone(), "invocation-2", Some(excluding.clone()));
    let (second_requests, second_exchanges) = serve(second, config);
    let (third, config) = membership_source(document.clone(), "invocation-3", Some(include));
    let (third_requests, third_exchanges) = serve(third, config);

    assert_eq!(
        [first_exchanges, second_exchanges, third_exchanges],
        [1, 1, 1],
        "every invocation must reach the provider exactly once"
    );
    assert_eq!(
        first_requests, third_requests,
        "invocation 3 must reproduce invocation 1's exact application bytes"
    );
    assert_ne!(
        first_requests, second_requests,
        "the exclusion must actually change the application bytes"
    );
    assert!(
        contains(&first_requests, b"beta") && contains(&third_requests, b"beta"),
        "the included invocation must carry the collected item"
    );
    assert!(
        !contains(&second_requests, b"beta"),
        "the excluded item must not appear in application bytes"
    );
    // Collection continues: the registry still holds every item, including the excluded one.
    assert_eq!(
        document.items.len(),
        3,
        "a selector must not narrow collection"
    );
}

// AC2: a sibling-scoped item is refused before dispatch, and an explicit authorized wider scope is
// admitted on this same path.
#[test]
fn scoped_items_are_refused_before_dispatch_unless_authorized() {
    let (mut document, references) = history_document();
    let scoped = ContextItem {
        reference: ContextItemRef {
            item_id: "episode-note-1".to_owned(),
            version: 1,
            sha256: crate::sha256_hex(b"sibling"),
        },
        kind: "episode_note".to_owned(),
        bytes: b"sibling".to_vec(),
        protected: false,
        expires_at: 100,
    };
    document
        .items
        .insert("episode-note-1:1".to_owned(), scoped.clone());
    document.draft.selected_items.push(scoped.reference.clone());

    let (source, config) = membership_source(
        document.clone(),
        "invocation-1",
        Some(ContextMembershipSelector::include()),
    );
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        ContextRenderLimits::harness_maxima(),
        Default::default(),
        None,
    );
    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("an unauthorized scoped item must be refused");
    assert!(
        error.message.contains("sibling_scope_leak"),
        "the refusal must name the scope gate: {}",
        error.message
    );
    assert_eq!(
        exchanges.load(Ordering::SeqCst),
        0,
        "a membership refusal must happen before any provider exchange"
    );

    let mut authorized = ContextMembershipSelector::include();
    authorized.broader_scope.items = vec![scoped.reference.clone()];
    authorized.broader_scope.authorized_agent_ids = vec!["test-agent".to_owned()];
    let (source, config) =
        membership_source(document.clone(), "invocation-1", Some(authorized.clone()));
    let (requests, exchanges) = serve(source, config);
    assert_eq!(exchanges, 1, "an authorized wider scope is admitted");
    assert!(
        contains(&requests, b"sibling"),
        "the authorized item must reach model-visible input"
    );
    assert_eq!(
        references.len(),
        3,
        "the shared items are unaffected by the wider scope"
    );
}

// AC3: protected exclusions and mandatory overflow fail before dispatch with precise reasons, and a
// retained prerequisite never reaches model-visible input.
#[test]
fn protected_and_overflow_gates_fail_before_dispatch() {
    let (mut document, references) = history_document();
    if let Some(item) = document.items.get_mut("history-1:1") {
        item.protected = true;
    }

    // Excluding an owner prerequisite is refused precisely, before dispatch.
    let excluding = ContextMembershipSelector {
        disposition: MembershipDisposition::Exclude,
        overrides: vec![references[1].clone()],
        ..ContextMembershipSelector::include()
    };
    let (source, config) = membership_source(document.clone(), "invocation-1", Some(excluding));
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        ContextRenderLimits::harness_maxima(),
        Default::default(),
        None,
    );
    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("excluding a prerequisite must be refused");
    assert!(
        error.message.contains("protected_prerequisite_excluded"),
        "the refusal must name the prerequisite gate: {}",
        error.message
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);

    // A retained prerequisite is suppressed from model-visible input, not published.
    let (source, config) = membership_source(
        document.clone(),
        "invocation-1",
        Some(ContextMembershipSelector::include()),
    );
    let (requests, exchanges) = serve(source, config);
    assert_eq!(exchanges, 1);
    assert!(
        !contains(&requests, b"beta"),
        "a retained prerequisite must stay out of model-visible input"
    );
    assert!(
        contains(&requests, b"alpha") && contains(&requests, b"gamma"),
        "the remaining model-visible items must still be published"
    );

    // A prerequisite alone over the selected bound is refused as a mandatory/pin overflow.
    let (source, config) = membership_source(
        document,
        "invocation-1",
        Some(ContextMembershipSelector::include()),
    );
    let (mut session, _, exchanges, _) =
        render_test_session(source, config, selected_limits(0), Default::default(), None);
    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a prerequisite over the bound must be refused");
    assert!(
        error.message.contains("mandatory_pin_overflow"),
        "the refusal must name the mandatory/pin gate: {}",
        error.message
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
}

// AC4: effective absence is refused for provider history a selector cannot erase.
#[test]
fn opaque_provider_history_rejects_effective_absence() {
    let (document, _) = history_document();
    let hidden = ContextMembershipSelector {
        model_view: ContextModelView {
            observation_visible: false,
        },
        ..ContextMembershipSelector::include()
    };
    let (mut source, config) = membership_source(document, "invocation-1", Some(hidden));
    source.continuity = MembershipContinuity::OpaquePersistent;
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        ContextRenderLimits::harness_maxima(),
        Default::default(),
        None,
    );
    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("an opaque provider cannot execute effective absence");
    assert!(
        error.message.contains("effective_absence_unsupported"),
        "the refusal must name the absence gate: {}",
        error.message
    );
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
}
