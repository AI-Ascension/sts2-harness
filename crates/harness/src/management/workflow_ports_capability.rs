// SPDX-License-Identifier: MIT

//! The synthetic capability port and the capability/catalog constants it advertises.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use serde_json::{Value, json};

use super::auth::AuthContext;
use super::contract::{
    ExecutionMode, TARGET_CATALOG_SCHEMA_VERSION, TargetAvailability, TargetCatalogResponse,
    TargetDescriptor,
};
use super::service::{CapabilityPort, ManagementError};

pub(super) struct SyntheticCapabilityPort;

impl CapabilityPort for SyntheticCapabilityPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({
            "schema_version": "ascension.capabilities/v1",
            "producer": "sts2-harness.synthetic",
            "profile": SYNTHETIC_TARGET_GAME_PROFILE,
            "capabilities": SYNTHETIC_CAPABILITY_IDS,
            "context_bindings": [
                {"context_ref": "context.synthetic.v1", "node_kinds": ["analyze", "decide"]},
                {"context_ref": "sts2.setup.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.map.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.combat.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.reward.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.shop.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.event.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.rest.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.selection.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.campaign.context.v1", "node_kinds": ["decide"]}
            ],
            "evidence_scope": "synthetic"
        }))
    }

    /// The synthetic owner serves the clearly-labelled synthetic inference
    /// catalog so discovery and exact resolution can be exercised with no
    /// provider, model or credential; it proves nothing about provider execution.
    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<Option<super::contract::InferenceProfileCatalog>, ManagementError> {
        super::synthetic_inference_profiles::synthetic_inference_profile_catalog().map(Some)
    }

    /// The synthetic owner publishes exactly one clearly-labelled synthetic
    /// target so authenticated consumers can preflight an exact admission
    /// without any game, provider, or lease authority.
    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(TargetCatalogResponse {
            schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
            catalog_revision: SYNTHETIC_TARGET_CATALOG_REVISION.to_owned(),
            targets: vec![TargetDescriptor {
                instance_id: SYNTHETIC_TARGET_INSTANCE_ID.to_owned(),
                execution_profiles: vec!["synthetic".to_owned()],
                execution_mode: ExecutionMode::Synthetic,
                compatibility_revision: SYNTHETIC_TARGET_COMPATIBILITY_REVISION.to_owned(),
                capability_revision: SYNTHETIC_TARGET_CAPABILITY_REVISION.to_owned(),
                availability: TargetAvailability::Available,
                supported_operations: vec![
                    "workflow:run".to_owned(),
                    "workflow:read".to_owned(),
                    "workflow:control".to_owned(),
                ],
                capabilities: SYNTHETIC_CAPABILITY_IDS
                    .iter()
                    .map(|capability| (*capability).to_owned())
                    .collect(),
                game_profiles: vec![SYNTHETIC_TARGET_GAME_PROFILE.to_owned()],
                save_profiles: Vec::new(),
                inference_profiles: Vec::new(),
            }],
        })
    }
}

/// Capabilities advertised by the deterministic synthetic owner. This list is
/// the single source for both the capability manifest and the run-target
/// descriptor so the two projections cannot drift.
const SYNTHETIC_CAPABILITY_IDS: &[&str] = &[
    "observe.fair-play.v1",
    "actions.catalog.v1",
    "actions.settlement.v1",
    "authority.generation-fence.v1",
    "actions.setup.v1",
    "actions.map.v1",
    "observe.map.v1",
    "actions.combat.v1",
    "actions.reward.v1",
    "actions.shop.v1",
    "actions.event.v1",
    "actions.rest.v1",
    "actions.selection.v1",
    "operations.reconcile.v1",
    "terminal.observation.v1",
    "analysis.combat.v1",
    "analysis.map.v1",
];

const SYNTHETIC_TARGET_INSTANCE_ID: &str = "sts2-synthetic-1";
const SYNTHETIC_TARGET_CATALOG_REVISION: &str = "synthetic.catalog.v1";
const SYNTHETIC_TARGET_COMPATIBILITY_REVISION: &str = "synthetic.compatibility.v1";
const SYNTHETIC_TARGET_CAPABILITY_REVISION: &str = "synthetic.capabilities.v1";
const SYNTHETIC_TARGET_GAME_PROFILE: &str = "sts2-synthetic-v1";
