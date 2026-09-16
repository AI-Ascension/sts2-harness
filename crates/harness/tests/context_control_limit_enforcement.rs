// SPDX-License-Identifier: MIT

//! Production enforcement of the selected context-control limits.
//!
//! The renderer and the control-transition authority have always enforced the **harness** maxima.
//! A composed owner binding can advertise less than that, and issue #95 requires the *selected*
//! values to be enforced at the point of use rather than only validated. These tests drive the
//! production management entry points (`prepare_context_render`,
//! `bind_context_control_authority`), which compose the run's current binding with the catalog
//! that admits it, so removing the wiring - not merely the mechanism - fails them.
//!
//! Synthetic owner and in-memory store only; no provider or game is launched.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic, dead_code)]

use std::collections::BTreeMap;

use serde_json::json;
use sts2_harness::{
    ContextBoundary, ContextDraft, ContextItem, ContextItemRef, ContextNote, ContextRenderer,
    ControlAuthority, ExoConfig, ManagedRenderInput, sha256_hex,
};

#[path = "support/context_owner_effective_limits.rs"]
mod fixture;

use fixture::{Scenario, actor, described_service, run_id, selected_limits};

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
    }
}

/// A draft that selects `selected_items` distinct registered items and one note.
fn fixtures(selected_items: usize) -> (ContextDraft, BTreeMap<String, ContextItem>, ExoConfig) {
    let mut registry = BTreeMap::new();
    let mut draft = ContextDraft::new("draft-1", "revision-1");
    for index in 0..selected_items {
        let bytes = format!("historical fixture {index}").into_bytes();
        let item = ContextItem {
            reference: ContextItemRef {
                item_id: format!("history-{index}"),
                version: 1,
                sha256: sha256_hex(&bytes),
            },
            kind: "history".to_owned(),
            bytes,
            protected: false,
            expires_at: 100,
        };
        registry.insert(
            format!("{}:{}", item.reference.item_id, item.reference.version),
            item.clone(),
        );
        draft.selected_items.push(item.reference.clone());
        if index == 0 {
            draft.notes.push(ContextNote {
                reference: item.reference,
                attributed_to: "operator-1".to_owned(),
            });
        }
    }
    let config = ExoConfig::new(REVISION, 64 * 1024, 1024, 1_000).expect("config");
    (draft, registry, config)
}

#[test]
fn the_production_render_point_refuses_a_draft_above_the_selected_item_limit() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let selected = selected_limits();
    let (draft, registry, config) = fixtures(selected.max_items as usize + 1);
    let error = service
        .prepare_context_render(
            &actor(),
            &run,
            &boundary(),
            render_input(),
            &draft,
            &registry,
            &config,
            1,
        )
        .expect_err("a draft above the selected item limit must be refused");
    assert_eq!(error.code, "context_render_limit_exceeded");
    assert!(
        error.message.contains("max_items"),
        "the refusal must name the limit: {}",
        error.message
    );
}

#[test]
fn the_production_render_point_refuses_a_render_the_harness_maxima_would_accept() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let (draft, registry, config) = fixtures(1);
    // The identical draft, boundary and configuration are acceptable under the harness maxima, so
    // the refusal below is caused by the *selected* limits and nothing else.
    ContextRenderer::enabled_at(&boundary(), render_input(), &draft, &registry, &config, 1)
        .expect("the harness maxima accept this draft");
    let error = service
        .prepare_context_render(
            &actor(),
            &run,
            &boundary(),
            render_input(),
            &draft,
            &registry,
            &config,
            1,
        )
        .expect_err("the selected context byte limit must refuse this draft");
    assert_eq!(error.code, "context_render_limit_exceeded");
    assert!(
        error.message.contains("max_context_bytes"),
        "the refusal must name the limit: {}",
        error.message
    );
}

#[test]
fn the_production_render_point_uses_the_selected_objective_limit() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let (draft, registry, config) = fixtures(1);
    let mut input = render_input();
    input.objective = "s".repeat(selected_limits().max_objective_bytes as usize * 2);
    // The objective is under the harness maximum, so the refusal below can only come from the
    // selected limit this owner advertised.
    ContextRenderer::enabled_at(&boundary(), input.clone(), &draft, &registry, &config, 1)
        .expect("the harness maxima accept this objective");
    let error = service
        .prepare_context_render(
            &actor(),
            &run,
            &boundary(),
            input,
            &draft,
            &registry,
            &config,
            1,
        )
        .expect_err("the selected objective limit must refuse this input");
    assert_eq!(error.code, "context_render_limit_exceeded");
    assert!(
        error.message.contains("max_objective_bytes"),
        "the refusal must name the limit: {}",
        error.message
    );
}

#[test]
fn the_production_control_transition_refuses_past_the_selected_event_bound() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let selected = selected_limits();
    let authority = service
        .bind_context_control_authority(
            &actor(),
            &run,
            ControlAuthority::new(boundary(), "revision-1"),
        )
        .expect("the selected control-event bound applies");
    assert_eq!(
        authority.max_control_events(),
        selected.max_control_events,
        "the authority must carry the selected bound, not the harness maximum"
    );
    let mut authority = authority;
    // An admitted-and-settled pair records two transitions.
    for index in 0..selected.max_control_events / 2 {
        let operation_id = format!("operation-{index}");
        authority
            .admit_operation(&operation_id, 1)
            .expect("operation admitted");
        authority
            .settle_operation(&operation_id)
            .expect("operation settled");
    }
    assert_eq!(
        authority.events().len() as u64,
        selected.max_control_events,
        "the authority must record exactly the selected number of transitions"
    );
    let journal = authority.export_journal().expect("journal");
    assert_eq!(
        authority.admit_operation("operation-overflow", 1),
        Err("context_control_events_exhausted".to_owned()),
        "the transition past the selected bound must be refused, not dropped silently"
    );
    assert_eq!(
        authority.events().len() as u64,
        selected.max_control_events,
        "the refused transition must not be retained"
    );
    assert_eq!(
        authority.export_journal().expect("journal"),
        journal,
        "a refusal at the bound must leave the plan, boundary and operation ledger unchanged"
    );
}

#[test]
fn the_production_control_binding_refuses_an_existing_over_bound_journal() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let selected = selected_limits();
    let mut authority = ControlAuthority::new(owner_boundary(&run), "revision-1");
    for index in 0..selected.max_control_events / 2 {
        let operation_id = format!("operation-{index}");
        authority
            .admit_operation(&operation_id, 1)
            .expect("admitted");
        authority.settle_operation(&operation_id).expect("settled");
    }
    let bounded = service
        .bind_context_control_authority(&actor(), &run, authority.clone())
        .expect("an existing journal exactly at the bound remains admissible");
    assert_eq!(bounded.events(), authority.events());
    authority
        .admit_operation("one-over", 1)
        .expect("harness bound");
    let journal = authority.export_journal().expect("journal");
    let error = service
        .bind_context_control_authority(&actor(), &run, authority.clone())
        .expect_err("existing retained events above the selected bound must be refused");
    assert_eq!(error.code, "context_control_events_exhausted");
    assert_eq!(authority.export_journal().expect("journal"), journal);
}

#[test]
fn the_selected_bound_also_governs_recovery_from_a_journal() {
    let selected = selected_limits();
    let mut authority = ControlAuthority::new(boundary(), "revision-1")
        .with_max_control_events(selected.max_control_events)
        .expect("the selected control-event bound applies");
    for index in 0..selected.max_control_events / 2 {
        let operation_id = format!("operation-{index}");
        authority
            .admit_operation(&operation_id, 1)
            .expect("operation admitted");
        authority
            .settle_operation(&operation_id)
            .expect("operation settled");
    }
    let journal = authority.export_journal().expect("journal");
    // The journal retains exactly the selected bound, so recovery under it is accepted...
    let recovered = ControlAuthority::recover_bounded(&journal, selected.max_control_events)
        .expect("a journal at the selected bound recovers");
    assert_eq!(recovered.max_control_events(), selected.max_control_events);
    // ...but a stricter bound refuses the same journal instead of saturating it silently.
    assert_eq!(
        ControlAuthority::recover_bounded(&journal, selected.max_control_events - 1),
        Err("context_control_events_exhausted".to_owned()),
        "a journal over the selected bound must be refused at the recovery boundary"
    );
}
