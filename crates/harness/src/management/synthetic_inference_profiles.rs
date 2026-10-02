// SPDX-License-Identifier: MIT

//! Deterministic synthetic inference-profile catalog.
//!
//! This is the clearly-labelled, non-authoritative catalog served by the
//! synthetic process driver so a consumer can discover exact profile identities
//! and exercise the read route without any provider, model or credential. It
//! is intentionally separate from the served-live producer and proves nothing
//! about provider execution.

use super::contract::{
    INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION, INFERENCE_PROFILE_SCHEMA_VERSION,
    InferenceProfileBudgets, InferenceProfileCatalog, InferenceProfileContinuity,
    InferenceProfileDescriptor, InferenceProfileGrants, InferenceProfileState,
};
use super::service::ManagementError;

pub const SYNTHETIC_INFERENCE_OWNER_ID: &str = "sts2-synthetic-inference-owner";
const SYNTHETIC_INFERENCE_OWNER_VERSION: &str = "1.0.0";

/// `(profile_id, node_kind, compatible context)`.
///
/// The first two entries are the owner's own labelled fixtures. The `sts2.*`
/// entries below them are NOT those fixtures: they are the references the
/// admitted Phase 1 workflow fixtures actually carry, so that a consumer can
/// exercise the owner's authoritative resolution end to end against the
/// documents it really publishes.
///
/// This list previously claimed that "the fixture definitions under
/// `conformance/workflow-v1` use exactly these references". That was false.
/// The Phase 1 fixtures the Studio publishes live in the Studio repo at
/// `contracts/accepted/phase1/workflows/`, and every one of their
/// `decision_profile_ref` / `planner_profile_ref` members names an `sts2.*`
/// profile that this catalog did not serve, so the owner refused them with
/// `inference_profile_unknown` and the consumer could never reach a
/// publication. The names below are the ones those documents really use.
///
/// One available revision per `profile_id` keeps floating resolution
/// unambiguous: `resolve_floating` refuses when several available revisions
/// match, so adding a second available revision for any of these would make
/// the very documents this catalog exists to admit unresolvable.
///
/// Every entry here remains a clearly-labelled synthetic descriptor with no
/// provider, no model in flight, no credential and no lease. Serving these
/// names proves the owner's resolution and admission fences work. It proves
/// nothing about Exo, about a provider, or about the game.
const SYNTHETIC_PROFILES: &[(&str, &str, &str)] = &[
    ("decision.synthetic.v1", "decide", "context.synthetic.v1"),
    ("planner.synthetic.v1", "adaptive_region", ""),
    // The `decide` references carried by the admitted Phase 1 workflow
    // fixtures, each paired with the context its own node declares.
    (
        "sts2.campaign.decision.v1",
        "decide",
        "sts2.campaign.context.v1",
    ),
    (
        "sts2.combat.decision.v1",
        "decide",
        "sts2.combat.context.v1",
    ),
    ("sts2.event.decision.v1", "decide", "sts2.event.context.v1"),
    ("sts2.map.decision.v1", "decide", "sts2.map.context.v1"),
    ("sts2.rest.decision.v1", "decide", "sts2.rest.context.v1"),
    (
        "sts2.reward.decision.v1",
        "decide",
        "sts2.reward.context.v1",
    ),
    (
        "sts2.selection.decision.v1",
        "decide",
        "sts2.selection.context.v1",
    ),
    ("sts2.setup.decision.v1", "decide", "sts2.setup.context.v1"),
    ("sts2.shop.decision.v1", "decide", "sts2.shop.context.v1"),
    // The `adaptive_region` references, which declare no context at all.
    ("sts2.combat.planner.v1", "adaptive_region", ""),
    ("sts2.map.planner.v1", "adaptive_region", ""),
];

fn descriptor(
    profile_id: &str,
    node_kind: &str,
    context_ref: &str,
) -> Result<InferenceProfileDescriptor, ManagementError> {
    InferenceProfileDescriptor {
        schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: profile_id.to_owned(),
        version: "1.0.0".to_owned(),
        digest: String::new(),
        adapter: "synthetic.provider.v1".to_owned(),
        requested_model: "synthetic.model.v1".to_owned(),
        resolved_model: None,
        prompt_revision: "synthetic.prompt.v1".to_owned(),
        settings_revision: "synthetic.settings.v1".to_owned(),
        supported_settings: vec![
            "max_provider_calls".to_owned(),
            "max_output_tokens".to_owned(),
        ],
        operations: vec![node_kind.to_owned()],
        node_kinds: vec![node_kind.to_owned()],
        context_compatibility: if context_ref.is_empty() {
            Vec::new()
        } else {
            vec![context_ref.to_owned()]
        },
        continuity: InferenceProfileContinuity::default(),
        effective_budgets: InferenceProfileBudgets {
            max_input_bytes: 128 * 1024,
            max_output_tokens: 4096,
            max_provider_calls: 64,
        },
        // Every descriptor this owner publishes is editable, so the admitted edit
        // route can be exercised end to end with no provider, model or credential.
        // It proves the journal and the authority split; it proves nothing about a
        // live owner's edit policy, which is that owner's.
        grants: InferenceProfileGrants {
            select: true,
            edit: true,
        },
        state: InferenceProfileState::Available,
    }
    .seal()
    .map_err(ManagementError::from)
}

/// The synthetic owner's sealed, validated catalog.
pub fn synthetic_inference_profile_catalog() -> Result<InferenceProfileCatalog, ManagementError> {
    let descriptors = SYNTHETIC_PROFILES
        .iter()
        .map(|(profile_id, node_kind, context_ref)| descriptor(profile_id, node_kind, context_ref))
        .collect::<Result<Vec<_>, _>>()?;
    let catalog = InferenceProfileCatalog {
        schema_version: INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: SYNTHETIC_INFERENCE_OWNER_ID.to_owned(),
        owner_version: SYNTHETIC_INFERENCE_OWNER_VERSION.to_owned(),
        catalog_digest: String::new(),
        descriptors,
    }
    .seal()?;
    catalog.validate()?;
    Ok(catalog)
}
