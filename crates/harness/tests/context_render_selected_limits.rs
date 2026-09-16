// SPDX-License-Identifier: MIT

//! Selected owner/profile context-control limits are enforced before preparation.
//!
//! The renderer has always enforced the *harness* maxima. A binding can advertise limits below
//! those maxima, and issue #95 requires that a value the selected owner cannot accept is refused
//! with a precise error before inference or retention instead of failing late.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::json;
use std::collections::BTreeMap;
use sts2_harness::{
    ContextBoundary, ContextDraft, ContextDraft as Draft, ContextEffectiveLimits, ContextItem,
    ContextItemRef, ContextNote, ContextRenderError, ContextRenderLimits, ContextRenderer,
    ExoConfig, ManagedRenderInput,
};

const REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        state_id: "combat-1".to_owned(),
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
        state_id: "combat-1".to_owned(),
        generation: 1,
        observation: json!({
            "state_id":"combat-1",
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

fn sha256(bytes: &[u8]) -> String {
    sts2_harness::sha256_hex(bytes)
}

fn fixtures() -> (ContextDraft, BTreeMap<String, ContextItem>, ExoConfig) {
    let bytes = b"historical fixture".to_vec();
    let item = ContextItem {
        reference: ContextItemRef {
            item_id: "history-1".to_owned(),
            version: 1,
            sha256: sha256(&bytes),
        },
        kind: "history".to_owned(),
        bytes,
        protected: false,
        expires_at: 100,
    };
    let mut registry = BTreeMap::new();
    registry.insert("history-1:1".to_owned(), item.clone());
    let mut draft = Draft::new("draft-1", "revision-1");
    draft.selected_items.push(item.reference.clone());
    draft.notes.push(ContextNote {
        reference: item.reference.clone(),
        attributed_to: "operator-1".to_owned(),
    });
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    (draft, registry, config)
}

fn render_with(
    limits: &ContextRenderLimits,
) -> Result<sts2_harness::PreparedContext, ContextRenderError> {
    let (draft, registry, config) = fixtures();
    ContextRenderer::enabled_at_with_limits(
        &boundary(),
        render_input(),
        &draft,
        &registry,
        &config,
        1,
        limits,
    )
}

#[test]
fn harness_maxima_accepts_the_standard_fixture() {
    render_with(&ContextRenderLimits::harness_maxima()).expect("within the harness maxima");
    // The pre-existing entry point is unchanged.
    let (draft, registry, config) = fixtures();
    ContextRenderer::enabled_at(&boundary(), render_input(), &draft, &registry, &config, 1)
        .expect("unchanged entry point");
}

#[test]
fn selected_item_limit_is_enforced_precisely() {
    let limits = ContextRenderLimits {
        max_items: 0,
        ..ContextRenderLimits::harness_maxima()
    };
    assert_eq!(
        render_with(&limits),
        Err(ContextRenderError::ExceedsSelectedLimit("max_items"))
    );
}

#[test]
fn selected_note_limit_is_enforced_precisely() {
    let limits = ContextRenderLimits {
        max_notes: 0,
        ..ContextRenderLimits::harness_maxima()
    };
    assert_eq!(
        render_with(&limits),
        Err(ContextRenderError::ExceedsSelectedLimit("max_notes"))
    );
}

#[test]
fn selected_objective_limit_is_enforced_precisely() {
    // The fixture objective is "survive" (7 bytes).
    let limits = ContextRenderLimits {
        max_objective_bytes: 1,
        ..ContextRenderLimits::harness_maxima()
    };
    assert_eq!(
        render_with(&limits),
        Err(ContextRenderError::ExceedsSelectedLimit(
            "max_objective_bytes"
        ))
    );
}

#[test]
fn selected_context_bytes_limit_is_enforced_precisely() {
    let limits = ContextRenderLimits {
        max_context_bytes: 1,
        ..ContextRenderLimits::harness_maxima()
    };
    assert_eq!(
        render_with(&limits),
        Err(ContextRenderError::ExceedsSelectedLimit(
            "max_context_bytes"
        ))
    );
}

#[test]
fn the_advertised_limits_bridge_narrows_without_widening() {
    assert_eq!(
        ContextEffectiveLimits::default().render_limits(),
        ContextRenderLimits::harness_maxima(),
        "the default descriptor limits are exactly the harness maxima"
    );
    let narrowed = ContextEffectiveLimits {
        max_notes: 0,
        ..ContextEffectiveLimits::default()
    };
    let limits = narrowed.render_limits();
    assert_eq!(limits.max_notes, 0);
    assert_eq!(
        limits.max_context_bytes,
        ContextRenderLimits::harness_maxima().max_context_bytes
    );
    assert_eq!(
        render_with(&limits),
        Err(ContextRenderError::ExceedsSelectedLimit("max_notes"))
    );
}

#[test]
fn the_legacy_global_bound_is_unchanged() {
    let oversized = vec![b'a'; sts2_harness::MAX_CONTEXT_BYTES + 1];
    assert_eq!(
        ContextRenderer::legacy(oversized, b"schema".to_vec(), b"config".to_vec()),
        Err(ContextRenderError::TooLarge),
        "the pre-existing global bound still reports TooLarge"
    );
}
