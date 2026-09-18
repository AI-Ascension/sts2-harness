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

/// `(profile_id, node_kind, compatible context)`; the fixture definitions
/// under `conformance/workflow-v1` use exactly these references.
const SYNTHETIC_PROFILES: &[(&str, &str, &str)] = &[
    ("decision.synthetic.v1", "decide", "context.synthetic.v1"),
    ("planner.synthetic.v1", "adaptive_region", ""),
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
        grants: InferenceProfileGrants {
            select: true,
            edit: false,
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
