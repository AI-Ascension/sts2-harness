// SPDX-License-Identifier: MIT

//! Deterministic synthetic context-owner adapter.
//!
//! This is the clearly-labelled, non-authoritative owner used by the synthetic
//! process driver so a consumer can discover and bind a scoped context
//! reference without any context-console, provider, or game authority. It is
//! intentionally separate from the real owner transport.

use super::auth::AuthContext;
use super::context_owner::{
    CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_CATALOG_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingRequest, ContextBindingState, ContextEffectiveLimits,
    ContextOwnerBinding, ContextOwnerPort, catalog_digest,
};
use super::service::ManagementError;
use crate::context_control::ContextBoundary;
use crate::sha256_hex;

const SYNTHETIC_OWNER_ID: &str = "sts2-synthetic-context-owner";
const SYNTHETIC_OWNER_VERSION: &str = "1.0.0";
const SYNTHETIC_INVOCATION_ID: &str = "synthetic.invocation.1";

/// The context references the synthetic owner advertises, matching the
/// synthetic capability manifest so discovery and binding cannot drift.
const SYNTHETIC_BINDINGS: &[(&str, &[&str])] = &[
    ("context.synthetic.v1", &["analyze", "decide"]),
    ("sts2.setup.context.v1", &["decide"]),
    ("sts2.map.context.v1", &["decide"]),
    ("sts2.combat.context.v1", &["decide"]),
    ("sts2.reward.context.v1", &["decide"]),
    ("sts2.shop.context.v1", &["decide"]),
    ("sts2.event.context.v1", &["decide"]),
    ("sts2.rest.context.v1", &["decide"]),
    ("sts2.selection.context.v1", &["decide"]),
    ("sts2.campaign.context.v1", &["decide"]),
];

fn synthetic_continuity() -> ContextBindingContinuity {
    ContextBindingContinuity {
        survives_controller_restart: false,
        receipt_recovery: false,
        provider_session_continuity: false,
    }
}

fn synthetic_grants() -> ContextBindingGrants {
    ContextBindingGrants {
        metadata_read: true,
        content_read: false,
        edit: false,
        control: false,
    }
}

fn descriptor_for(
    context_ref: &str,
    node_kinds: &[&str],
) -> Result<ContextBindingDescriptor, ManagementError> {
    ContextBindingDescriptor {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        binding_id: format!("synthetic.{context_ref}.binding.v1"),
        version: 1,
        digest: String::new(),
        context_ref: context_ref.to_owned(),
        node_kinds: node_kinds.iter().map(|kind| (*kind).to_owned()).collect(),
        sources: Vec::new(),
        operations: Vec::new(),
        effective_limits: ContextEffectiveLimits::default(),
        continuity: synthetic_continuity(),
        grants: synthetic_grants(),
        state: ContextBindingState::Available,
    }
    .seal()
}

fn descriptors() -> Result<Vec<ContextBindingDescriptor>, ManagementError> {
    let mut descriptors = SYNTHETIC_BINDINGS
        .iter()
        .map(|(context_ref, node_kinds)| descriptor_for(context_ref, node_kinds))
        .collect::<Result<Vec<_>, _>>()?;
    descriptors.sort_by(|left, right| left.context_ref.cmp(&right.context_ref));
    Ok(descriptors)
}

fn descriptor_for_request(
    request: &ContextBindingRequest,
) -> Result<ContextBindingDescriptor, ManagementError> {
    let descriptor = descriptors()?
        .into_iter()
        .find(|descriptor| {
            descriptor.context_ref == request.context_ref
                && descriptor
                    .node_kinds
                    .iter()
                    .any(|kind| kind == &request.node_kind)
        })
        .ok_or_else(|| {
            ManagementError::capability(
                "context_binding_unsupported",
                "synthetic context owner does not advertise the requested binding",
            )
        })?;
    if request.binding_id != descriptor.binding_id
        || request.binding_version != descriptor.version
        || request.binding_digest != descriptor.digest
    {
        return Err(ManagementError::conflict(
            "context_binding_stale",
            "synthetic context owner binding identity does not match the catalog",
        ));
    }
    Ok(descriptor)
}

fn synthetic_boundary(run_id: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: run_id.to_owned(),
        episode_id: "synthetic.episode.1".to_owned(),
        agent_id: "synthetic.agent.1".to_owned(),
        state_id: "synthetic.state.1".to_owned(),
        generation: 1,
        observation_sha256: sha256_hex(format!("synthetic.observation.{run_id}")),
        catalog_sha256: sha256_hex("synthetic.context.catalog"),
        adapter_revision: "synthetic.adapter.v1".to_owned(),
        model_revision: "synthetic.model.v1".to_owned(),
        configuration_sha256: sha256_hex("synthetic.context.configuration"),
        output_schema_sha256: sha256_hex("synthetic.context.output-schema"),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

/// Synthetic owner that advertises metadata-only synthetic bindings.
pub struct SyntheticContextOwnerPort;

impl ContextOwnerPort for SyntheticContextOwnerPort {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        let descriptors = descriptors()?;
        let catalog = ContextBindingCatalog {
            schema_version: CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.to_owned(),
            owner_id: SYNTHETIC_OWNER_ID.to_owned(),
            owner_version: SYNTHETIC_OWNER_VERSION.to_owned(),
            catalog_digest: catalog_digest(
                SYNTHETIC_OWNER_ID,
                SYNTHETIC_OWNER_VERSION,
                &descriptors,
            )?,
            descriptors,
        };
        catalog.validate()?;
        Ok(catalog)
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        descriptor_for_request(request)?;
        let binding = ContextOwnerBinding {
            schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
            owner_id: SYNTHETIC_OWNER_ID.to_owned(),
            owner_version: SYNTHETIC_OWNER_VERSION.to_owned(),
            invocation_id: SYNTHETIC_INVOCATION_ID.to_owned(),
            binding_id: request.binding_id.clone(),
            binding_version: request.binding_version,
            binding_digest: request.binding_digest.clone(),
            context_ref: request.context_ref.clone(),
            instance_id: request.instance_id.clone(),
            node_kind: request.node_kind.clone(),
            state: ContextBindingState::Available,
            workflow_run_id: request.workflow_run_id.clone(),
            definition_digest: request.definition_digest.clone(),
            graph_id: request.graph_id.clone(),
            node_id: request.node_id.clone(),
            node_execution_id: request.node_execution_id.clone(),
            boundary: synthetic_boundary(&request.workflow_run_id),
            lease_epoch: 1,
            snapshot_id: "synthetic.snapshot.1".to_owned(),
            approved_revision_id: "synthetic.revision.1".to_owned(),
            plan_epoch: 1,
            grants: synthetic_grants(),
            continuity: synthetic_continuity(),
        };
        binding.validate(None)?;
        Ok(binding)
    }
}
