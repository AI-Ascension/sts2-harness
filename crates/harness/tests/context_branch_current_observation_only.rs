// SPDX-License-Identifier: MIT

//! #118 AC2 — the named current-observation-only branch-context mode.
//!
//! A child invocation rendered under the default *through-fork* mode carries its permitted
//! ancestor item; the same child rendered under the explicitly selected *current-observation-only*
//! mode omits every ancestor item (and ancestor note) while still carrying the live observation.
//! The chosen mode is recorded on the effective membership, the dispatch view and the policy
//! digest, so a later invocation can prove which mode produced its bytes.
//!
//! The final test shows an explicit model-revision change is recorded in the prepared configuration
//! bytes; prompt-revision edits are recorded by the inference-profile revision journal
//! (`crates/harness/tests/inference_profile_revision.rs`).
//!
//! Synthetic fixtures only: no provider, native process, host, game or wall clock is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::json;
use std::collections::BTreeMap;

use sts2_harness::context_capture::{
    CaptureComponent, CaptureComponentKind, PreparedApplicationInput,
};
use sts2_harness::context_control::{
    AncestorHistoryMode, ContextMembershipSelector, MembershipContinuity, MembershipRenderRequest,
    render_with_membership,
};
use sts2_harness::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextNote, ContextRenderLimits,
    ContextSourceDocument, ExoConfig, ManagedRenderInput, PreparedContext,
};

const REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";
const NOW: u64 = 500;
const ANCESTOR_MARKER: &str = "MARKER-ANCESTOR-THROUGH-FORK-4c7d";
const OBSERVATION_MARKER: &str = "OBS-MARKER-71ac";

fn boundary_with_model(model_revision: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: "run-child".to_owned(),
        episode_id: "episode-child".to_owned(),
        agent_id: "agent-child".to_owned(),
        state_id: "combat-child".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: REVISION.to_owned(),
        model_revision: model_revision.to_owned(),
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
            "visible_seed": OBSERVATION_MARKER,
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

fn ancestor_document() -> (ContextSourceDocument, ContextItemRef) {
    let bytes = ANCESTOR_MARKER.as_bytes().to_vec();
    let item = ContextItem {
        reference: ContextItemRef {
            item_id: "history-ancestor".to_owned(),
            version: 1,
            sha256: sts2_harness::sha256_hex(&bytes),
        },
        kind: "history".to_owned(),
        bytes,
        protected: false,
        expires_at: NOW + 10_000,
    };
    let reference = item.reference.clone();

    let mut items = BTreeMap::new();
    items.insert("history-ancestor:1".to_owned(), item.clone());

    let mut draft = ContextDraft::new("draft-child", "revision-child");
    draft.selected_items.push(item.reference.clone());
    draft.notes.push(ContextNote {
        reference: item.reference.clone(),
        attributed_to: "operator-1".to_owned(),
    });
    draft.pinned_item_ids.push("history-ancestor".to_owned());

    (ContextSourceDocument { draft, items }, reference)
}

fn render(
    document: &ContextSourceDocument,
    config: &ExoConfig,
    model_revision: &str,
    selector: &ContextMembershipSelector,
) -> (
    PreparedContext,
    sts2_harness::context_control::PreparedMembership,
) {
    let policy = selector.bind("invocation-child", "revision-child");
    let (prepared, membership) = render_with_membership(MembershipRenderRequest {
        boundary: &boundary_with_model(model_revision),
        request: render_input(),
        document,
        config,
        now: NOW,
        limits: &ContextRenderLimits::harness_maxima(),
        policy: Some(&policy),
        continuity: MembershipContinuity::Stateless,
    })
    .expect("child render");
    (prepared, membership.expect("prepared membership"))
}

fn serialized(prepared: &PreparedContext) -> Vec<u8> {
    let components = [CaptureComponent {
        kind: CaptureComponentKind::UserMessage,
        ordinal: 1,
        media_type: "application/json",
        bytes: prepared.provider_bytes(),
    }];
    PreparedApplicationInput::prepare("exo", "model-execution-7", None, &components)
        .expect("prepared application input")
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

// AC2: current-observation-only omits ancestor history from the serialized input while the
// observation is still carried, and the mode is recorded.
#[test]
fn current_observation_only_omits_ancestor_history_and_records_the_mode() {
    let (document, reference) = ancestor_document();
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let selector = ContextMembershipSelector::current_observation_only();

    let (prepared, membership) = render(&document, &config, "model-v1", &selector);

    assert_eq!(
        membership.effective.ancestor_history,
        AncestorHistoryMode::CurrentObservationOnly,
        "the resolved set must record the current-observation-only mode"
    );
    assert_eq!(
        membership.dispatch_view.ancestor_history,
        AncestorHistoryMode::CurrentObservationOnly,
        "the dispatch view must record the mode a later invocation revalidates against"
    );
    assert!(
        membership.effective.model_visible.is_empty(),
        "no ancestor item may remain model-visible"
    );
    assert_eq!(
        membership.effective.included,
        vec![reference],
        "the ancestor item stays retained for owner legality even when omitted from model input"
    );

    let bytes = serialized(&prepared);
    assert!(
        contains(&bytes, OBSERVATION_MARKER),
        "current-observation-only must still carry the live observation"
    );
    assert!(
        !contains(&bytes, ANCESTOR_MARKER),
        "current-observation-only must omit ancestor history from the serialized input"
    );

    // The mode is bound into the policy digest, so a revalidation proves the exact mode.
    let through_fork = ContextMembershipSelector::include()
        .bind("invocation-child", "revision-child")
        .digest()
        .expect("digest");
    let only = selector
        .bind("invocation-child", "revision-child")
        .digest()
        .expect("digest");
    assert_ne!(
        through_fork, only,
        "the recorded policy digest must distinguish the two ancestor-history modes"
    );
}

// Control: through-fork (default) carries the permitted ancestor item, proving the negative
// assertion above is a mode choice rather than the harness never serializing ancestors.
#[test]
fn through_fork_control_carries_the_permitted_ancestor() {
    let (document, reference) = ancestor_document();
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let selector = ContextMembershipSelector::include();

    let (prepared, membership) = render(&document, &config, "model-v1", &selector);
    assert_eq!(
        membership.effective.ancestor_history,
        AncestorHistoryMode::ThroughFork
    );
    assert_eq!(membership.effective.model_visible, vec![reference]);
    let bytes = serialized(&prepared);
    assert!(
        contains(&bytes, ANCESTOR_MARKER),
        "the default through-fork mode must carry the permitted ancestor item"
    );
    assert!(contains(&bytes, OBSERVATION_MARKER));
}

// AC2: an explicit model-revision change is recorded in the prepared configuration bytes (and the
// prompt/settings revision axis is recorded by the inference-profile revision journal).
#[test]
fn explicit_model_revision_changes_are_recorded() {
    let (document, _reference) = ancestor_document();
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    let selector = ContextMembershipSelector::include();

    let (first, _) = render(&document, &config, "model-v1", &selector);
    let (second, _) = render(&document, &config, "model-v2", &selector);

    let first_config = String::from_utf8(first.configuration.clone()).expect("utf8 configuration");
    let second_config =
        String::from_utf8(second.configuration.clone()).expect("utf8 configuration");
    assert!(
        first_config.contains("model-v1"),
        "the recorded configuration must name the model revision"
    );
    assert!(second_config.contains("model-v2"));
    assert_ne!(
        first_config, second_config,
        "a change to the explicit model revision must change the recorded configuration"
    );
    assert_ne!(
        first.manifest_sha256, second.manifest_sha256,
        "a model-revision change must change the prepared manifest"
    );
}
