// SPDX-License-Identifier: MIT

//! Synthetic inference-profile fixtures shared by the catalog admission suite.
//!
//! Every catalog, descriptor and definition here is in-memory and labelled
//! synthetic; no provider, model, credential or native host is contacted.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use serde_json::{Value, json};
use sts2_harness::management::{
    INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION, INFERENCE_PROFILE_SCHEMA_VERSION,
    InferenceProfileBudgets, InferenceProfileCatalog, InferenceProfileContinuity,
    InferenceProfileDescriptor, InferenceProfileGrants, InferenceProfileState,
    inference_catalog_digest,
};

pub(super) const CATALOG_REVISION: &str = "synthetic.catalog.v1";

const OWNER_ID: &str = "synthetic.inference.owner";

/// A live definition with two inference nodes: an `adaptive_region` planner and
/// a `decide` node, each referencing its own profile. It is deliberately a
/// different topology from the single-node fixture so one submission must bind
/// two distinct revisions.
pub(crate) fn two_node_definition(decide_ref: &str, planner_ref: &str) -> Value {
    json!({
        "schema_version": "ascension.workflow/v1",
        "workflow_id": "fixture.inference.two-node",
        "version": "0.1.0",
        "mode": "dynamic",
        "game_profile": "sts2-live-v1",
        "policy_ref": "policy.live.v1",
        "capabilities": {"required": [], "optional": []},
        "limits": {
            "max_steps": 32,
            "max_subworkflow_depth": 3,
            "max_provider_calls": 8,
            "max_parallel_analyses": 1,
            "max_output_tokens": 4096
        },
        "entry_graph": "main",
        "graphs": [{
            "id": "main",
            "entry_node": "decide",
            "nodes": [
                {
                    "id": "decide",
                    "kind": "decide",
                    "config": {
                        "decision_profile_ref": decide_ref,
                        "context_ref": "context.live.v1"
                    }
                },
                {
                    "id": "plan",
                    "kind": "adaptive_region",
                    "config": {
                        "region_id": "region-1",
                        "planner_profile_ref": planner_ref,
                        "allowed_operations": ["observe.fair-play.v1"],
                        "max_plan_nodes": 4,
                        "max_plan_edges": 4,
                        "max_replans": 2,
                        "output_type": "DecisionProposal"
                    }
                },
                {"id": "done", "kind": "terminal", "config": {"outcome": "completed"}}
            ],
            "edges": [
                {"from": "decide", "to": "plan", "on": "ok", "priority": 0},
                {"from": "plan", "to": "done", "on": "ok", "priority": 0}
            ]
        }],
        "annotations": {"summary": "Synthetic two-node inference fixture.", "synthetic": true}
    })
}

pub(crate) fn descriptor(
    profile_id: &str,
    node_kind: &str,
    context_refs: &[&str],
    state: InferenceProfileState,
    select: bool,
) -> InferenceProfileDescriptor {
    descriptor_with_grants(profile_id, node_kind, context_refs, state, select, false)
}

fn descriptor_with_grants(
    profile_id: &str,
    node_kind: &str,
    context_refs: &[&str],
    state: InferenceProfileState,
    select: bool,
    edit: bool,
) -> InferenceProfileDescriptor {
    InferenceProfileDescriptor {
        schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: profile_id.to_owned(),
        version: "1.0.0".to_owned(),
        digest: String::new(),
        adapter: "synthetic.provider.v1".to_owned(),
        requested_model: "synthetic.model.v1".to_owned(),
        resolved_model: Some("synthetic.model.v1.observed".to_owned()),
        prompt_revision: "synthetic.prompt.v1".to_owned(),
        settings_revision: "synthetic.settings.v1".to_owned(),
        supported_settings: vec![
            "max_provider_calls".to_owned(),
            "max_output_tokens".to_owned(),
        ],
        operations: vec![node_kind.to_owned()],
        node_kinds: vec![node_kind.to_owned()],
        context_compatibility: context_refs
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        continuity: InferenceProfileContinuity::default(),
        effective_budgets: InferenceProfileBudgets {
            max_input_bytes: 128 * 1024,
            max_output_tokens: 4096,
            max_provider_calls: 64,
        },
        grants: InferenceProfileGrants { select, edit },
        state,
    }
    .seal()
    .expect("seal descriptor")
}

pub(crate) fn available(
    profile_id: &str,
    node_kind: &str,
    context_refs: &[&str],
) -> InferenceProfileDescriptor {
    descriptor(
        profile_id,
        node_kind,
        context_refs,
        InferenceProfileState::Available,
        true,
    )
}

/// `available`, published by its owner as editable.
///
/// The admitted edit route requires `grants.edit` in addition to the caller's
/// write scope, so the edit suite needs a fixture whose owner publishes that
/// grant. Every other fixture in this module stays `edit: false`, which is what
/// `catalog_with_decide` asserts.
pub(crate) fn available_editable(
    profile_id: &str,
    node_kind: &str,
    context_refs: &[&str],
) -> InferenceProfileDescriptor {
    descriptor_with_grants(
        profile_id,
        node_kind,
        context_refs,
        InferenceProfileState::Available,
        true,
        true,
    )
}

/// The two-node baseline, with the `decide` profile published as editable and
/// the planner left uneditable, so one catalog exercises both authority halves.
pub(crate) fn editable_catalog() -> InferenceProfileCatalog {
    catalog(vec![
        available_editable("decision.synthetic.v1", "decide", &["context.live.v1"]),
        available("planner.synthetic.v1", "adaptive_region", &[]),
    ])
}

pub(crate) fn catalog(descriptors: Vec<InferenceProfileDescriptor>) -> InferenceProfileCatalog {
    let owner_version = "1.0.0".to_owned();
    let catalog_digest =
        inference_catalog_digest(OWNER_ID, &owner_version, &descriptors).expect("catalog digest");
    InferenceProfileCatalog {
        schema_version: INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: OWNER_ID.to_owned(),
        owner_version,
        catalog_digest,
        descriptors,
    }
}

/// Both nodes of the two-node fixture resolve to distinct available revisions.
pub(crate) fn baseline_catalog() -> InferenceProfileCatalog {
    catalog(vec![
        available("decision.synthetic.v1", "decide", &["context.live.v1"]),
        available("planner.synthetic.v1", "adaptive_region", &[]),
    ])
}

pub(crate) fn catalog_with_decide(
    state: InferenceProfileState,
    select: bool,
) -> InferenceProfileCatalog {
    catalog(vec![
        descriptor(
            "decision.synthetic.v1",
            "decide",
            &["context.live.v1"],
            state,
            select,
        ),
        available("planner.synthetic.v1", "adaptive_region", &[]),
    ])
}
