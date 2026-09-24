// SPDX-License-Identifier: MIT

//! #118 AC1 — adversarial cross-branch marker isolation through the production render path.
//!
//! A fresh child invocation is reconstructed under the default include selector while the shared
//! collection still holds uniquely marked parent post-fork history, a sibling outcome, a stale
//! ancestor summary, a sibling retrieval entry, sibling native provider history and a sibling pin.
//! The child's own permitted ancestor item is the positive control.
//!
//! The suite asserts on the *actual serialized prepared input* — the bytes the harness would hand a
//! provider boundary and the exact components of a prepared application input — not on membership
//! metadata. It fails if the render publishes anything outside the invocation's model-visible set,
//! or if a non-shared sibling item can be smuggled in by an explicit policy.
//!
//! Synthetic fixtures only: no provider, native process, host, game or wall clock is contacted, so
//! this is component evidence for the render/scope seam, not live-inference evidence.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::json;
use std::collections::BTreeMap;

use sts2_harness::context_capture::{
    CaptureComponent, CaptureComponentKind, PreparedApplicationInput,
};
use sts2_harness::context_control::{
    ContextMembershipError, ContextMembershipScope, ContextMembershipSelector,
    MembershipContinuity, MembershipRenderRequest, render_with_membership, resolve_membership,
};
use sts2_harness::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextRenderLimits,
    ContextSourceDocument, ExoConfig, ManagedRenderInput, PreparedContext,
};

const REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";
const NOW: u64 = 500;

// Unique per-vector markers. Each must be absent from a fresh child's serialized input. The
// permitted marker must be present, so a negative assertion cannot pass by the harness silently
// serializing nothing.
const MARKER_ANCESTOR_PERMITTED: &str = "MARKER-ANCESTOR-PERMITTED-0a1b";
const MARKER_PARENT_POSTFORK_HISTORY: &str = "MARKER-PARENT-POSTFORK-HISTORY-2f3a";
const MARKER_SIBLING_OUTCOME: &str = "MARKER-SIBLING-OUTCOME-7b1c";
const MARKER_STALE_ANCESTOR_SUMMARY: &str = "MARKER-ANCESTOR-SUMMARY-STALE-9d4e";
const MARKER_SIBLING_RETRIEVAL: &str = "MARKER-SIBLING-RETRIEVAL-1a6f";
const MARKER_NATIVE_PROVIDER_HISTORY: &str = "MARKER-NATIVE-PROVIDER-HISTORY-5c8b";
const MARKER_SIBLING_PIN: &str = "MARKER-SIBLING-PIN-6e2d";

/// The markers that must never appear in a fresh child's serialized prepared input.
const FORBIDDEN_MARKERS: [&str; 6] = [
    MARKER_PARENT_POSTFORK_HISTORY,
    MARKER_SIBLING_OUTCOME,
    MARKER_STALE_ANCESTOR_SUMMARY,
    MARKER_SIBLING_RETRIEVAL,
    MARKER_NATIVE_PROVIDER_HISTORY,
    MARKER_SIBLING_PIN,
];

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: "run-child".to_owned(),
        episode_id: "episode-child".to_owned(),
        agent_id: "agent-child".to_owned(),
        state_id: "combat-child".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: REVISION.to_owned(),
        model_revision: "model-v1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 0,
    }
}

fn render_input() -> ManagedRenderInput {
    ManagedRenderInput {
        execution_id: "model-execution-7".to_owned(),
        state_id: "combat-child".to_owned(),
        generation: 1,
        observation: json!({
            "state_id":"combat-child",
            "generation":1,
            "visible_seed":"fixture-seed",
            "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
        legal_action_ids: vec!["combat.end-turn".to_owned()],
        objective: "survive".to_owned(),
        hard_constraints: vec!["visible state only".to_owned()],
        map_context: None,
    }
}

fn item(item_id: &str, version: u64, kind: &str, content: &str) -> ContextItem {
    let bytes = content.as_bytes().to_vec();
    ContextItem {
        reference: ContextItemRef {
            item_id: item_id.to_owned(),
            version,
            sha256: sts2_harness::sha256_hex(&bytes),
        },
        kind: kind.to_owned(),
        bytes,
        protected: false,
        expires_at: NOW + 10_000,
    }
}

/// The child's draft: it selects only the permitted ancestor item and pins a sibling pin marker.
/// The shared collection additionally holds every adversarial marker, so the render must publish
/// only the selection rather than the whole registry.
fn child_document() -> (ContextSourceDocument, ContextItemRef) {
    let permitted = item("history-ancestor", 1, "history", MARKER_ANCESTOR_PERMITTED);
    let adversarial = [
        item(
            "history-postfork",
            1,
            "history",
            MARKER_PARENT_POSTFORK_HISTORY,
        ),
        item("outcome-sibling", 1, "outcome", MARKER_SIBLING_OUTCOME),
        item(
            "summary-ancestor",
            1,
            "summary",
            MARKER_STALE_ANCESTOR_SUMMARY,
        ),
        item(
            "retrieval-sibling",
            1,
            "retrieval",
            MARKER_SIBLING_RETRIEVAL,
        ),
        item(
            "provider-history-sibling",
            1,
            "provider_history",
            MARKER_NATIVE_PROVIDER_HISTORY,
        ),
    ];

    let mut items = BTreeMap::new();
    items.insert(
        format!(
            "{}:{}",
            permitted.reference.item_id, permitted.reference.version
        ),
        permitted.clone(),
    );
    for entry in adversarial {
        items.insert(
            format!("{}:{}", entry.reference.item_id, entry.reference.version),
            entry,
        );
    }

    let mut draft = ContextDraft::new("draft-child", "revision-child");
    draft.selected_items.push(permitted.reference.clone());
    draft.pinned_item_ids.push(MARKER_SIBLING_PIN.to_owned());

    (ContextSourceDocument { draft, items }, permitted.reference)
}

fn serialized_components(prepared: &PreparedContext) -> Vec<u8> {
    let components = [CaptureComponent {
        kind: CaptureComponentKind::UserMessage,
        ordinal: 1,
        media_type: "application/json",
        bytes: prepared.provider_bytes(),
    }];
    let input = PreparedApplicationInput::prepare("exo", "model-execution-7", None, &components)
        .expect("prepared application input");
    input
        .components()
        .iter()
        .flat_map(|component| component.bytes().to_vec())
        .collect()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// AC1: no adversarial cross-branch marker reaches a fresh child's serialized prepared input, while
// the child's permitted ancestor item is serialized and the collection stays intact.
#[test]
fn adversarial_marks_are_absent_from_a_fresh_childs_prepared_input() {
    let (document, permitted) = child_document();
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let policy = ContextMembershipSelector::include().bind("invocation-child", "revision-child");

    let (prepared, membership) = render_with_membership(MembershipRenderRequest {
        boundary: &boundary(),
        request: render_input(),
        document: &document,
        config: &config,
        now: NOW,
        limits: &ContextRenderLimits::harness_maxima(),
        policy: Some(&policy),
        continuity: MembershipContinuity::Stateless,
    })
    .expect("fresh child render under the default include selector");

    assert_eq!(
        membership
            .as_ref()
            .expect("prepared membership")
            .effective
            .model_visible,
        vec![permitted],
        "the child must carry exactly its permitted ancestor item"
    );

    let serialized = serialized_components(&prepared);
    assert!(
        contains(&serialized, MARKER_ANCESTOR_PERMITTED),
        "positive control: the permitted ancestor item must be serialized, proving the harness \
         actually writes selected content"
    );
    for marker in FORBIDDEN_MARKERS {
        assert!(
            !contains(&serialized, marker),
            "adversarial marker {marker} must not appear in the fresh child's prepared input"
        );
    }

    // The exclusion is a scope/render property, not deletion: the collection still holds every
    // item-backed adversarial marker, and the sibling pin is still registered on the draft.
    for marker in [
        MARKER_PARENT_POSTFORK_HISTORY,
        MARKER_SIBLING_OUTCOME,
        MARKER_STALE_ANCESTOR_SUMMARY,
        MARKER_SIBLING_RETRIEVAL,
        MARKER_NATIVE_PROVIDER_HISTORY,
    ] {
        assert!(
            document
                .items
                .values()
                .any(|entry| entry.bytes == marker.as_bytes()),
            "collection must retain marker {marker}"
        );
    }
    assert!(
        document
            .draft
            .pinned_item_ids
            .iter()
            .any(|pin| pin == MARKER_SIBLING_PIN),
        "the sibling pin must stay registered on the draft even though it is never serialized"
    );
}

// AC1 negative control: a non-shared sibling marker cannot be smuggled in by an explicit policy;
// the scope gate refuses it before any bytes exist.
#[test]
fn explicit_policy_cannot_select_a_sibling_scoped_marker() {
    let (document, _permitted) = child_document();
    let sibling = document
        .items
        .get("outcome-sibling:1")
        .expect("sibling outcome registered");

    let mut selector = ContextMembershipSelector::include();
    selector.overrides = vec![sibling.reference.clone()];
    let policy = selector.bind("invocation-child", "revision-child");

    let result = resolve_membership(
        &policy,
        &document.draft,
        &document.items,
        &ContextMembershipScope {
            run_id: "run-child".to_owned(),
            episode_id: "episode-child".to_owned(),
            agent_id: "agent-child".to_owned(),
            branch_id: None,
        },
        NOW,
    );
    assert_eq!(
        result,
        Err(ContextMembershipError::SiblingScopeLeak {
            item_id: "outcome-sibling".to_owned()
        }),
        "an invocation-scoped sibling item must be refused without a wider scope"
    );
}
