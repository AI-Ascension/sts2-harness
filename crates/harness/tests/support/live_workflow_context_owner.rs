// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::sync::Arc;

use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_CATALOG_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingRequest, ContextBindingState, ContextEffectiveLimits,
    ContextOwnerBinding, ContextOwnerPort, LiveWorkflowOptions, LiveWorkflowSessionFactory,
    ManagementError, ManagementService, WorkflowStore, live_store,
};
use sts2_harness::sha256_hex;

/// Test double for the authoritative context owner. It publishes one available
/// synthetic binding so live admission can be exercised with an attached owner,
/// while the required production binding semantics are proven separately.
pub(crate) struct FakeContextOwner;

fn fake_context_descriptor() -> ContextBindingDescriptor {
    ContextBindingDescriptor {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        binding_id: "fake.binding.1".to_owned(),
        version: 1,
        digest: String::new(),
        context_ref: "context.live.v1".to_owned(),
        node_kinds: vec!["analyze".to_owned(), "decide".to_owned()],
        sources: Vec::new(),
        operations: Vec::new(),
        effective_limits: ContextEffectiveLimits::default(),
        continuity: ContextBindingContinuity {
            survives_controller_restart: false,
            receipt_recovery: false,
            provider_session_continuity: false,
        },
        grants: ContextBindingGrants {
            metadata_read: true,
            content_read: false,
            edit: false,
            control: false,
        },
        state: ContextBindingState::Available,
    }
    .seal()
    .expect("seal fake context descriptor")
}

fn fake_context_boundary(run_id: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: run_id.to_owned(),
        episode_id: "fake.episode.1".to_owned(),
        agent_id: "fake.agent.1".to_owned(),
        state_id: "fake.state.1".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "fake.adapter.v1".to_owned(),
        model_revision: "fake.model.v1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

impl ContextOwnerPort for FakeContextOwner {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        let owner_id = "fake.context-owner".to_owned();
        let owner_version = "1.0.0".to_owned();
        let descriptors = vec![fake_context_descriptor()];
        let bytes =
            serde_json::to_vec(&(&owner_id, &owner_version, &descriptors)).map_err(|error| {
                ManagementError::invalid("context_catalog_encode", error.to_string())
            })?;
        let catalog = ContextBindingCatalog {
            schema_version: CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.to_owned(),
            owner_id,
            owner_version,
            catalog_digest: sha256_hex(bytes),
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
        let binding = ContextOwnerBinding {
            schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
            owner_id: "fake.context-owner".to_owned(),
            owner_version: "1.0.0".to_owned(),
            invocation_id: "fake.invocation.1".to_owned(),
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
            boundary: fake_context_boundary(&request.workflow_run_id),
            lease_epoch: 1,
            snapshot_id: "fake.snapshot.1".to_owned(),
            approved_revision_id: "fake.revision.1".to_owned(),
            plan_epoch: 1,
            grants: ContextBindingGrants {
                metadata_read: true,
                content_read: false,
                edit: false,
                control: false,
            },
            continuity: ContextBindingContinuity {
                survives_controller_restart: false,
                receipt_recovery: false,
                provider_session_continuity: false,
            },
        };
        binding.validate(None)?;
        Ok(binding)
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Live service with the authoritative context owner attached, as required for
/// a live invocation.
pub(crate) fn live_service(
    store: Arc<dyn WorkflowStore>,
    factory: Arc<dyn LiveWorkflowSessionFactory>,
    options: LiveWorkflowOptions,
) -> Result<ManagementService, ManagementError> {
    live_store(store, factory, options)
        .map(|service| service.with_context_owner_port(Arc::new(FakeContextOwner)))
}
