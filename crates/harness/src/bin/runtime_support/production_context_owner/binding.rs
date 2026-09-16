// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::StoreMode;

impl Owner {
    pub(super) fn configuration_from_environment() -> Result<Configuration, String> {
        let raw = std::env::var("STS2_WORKFLOW_CONTEXT_OWNER_CONFIG")
            .map_err(|_| String::from("STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is required"))?;
        let value: Configuration = serde_json::from_str(&raw)
            .map_err(|_| String::from("STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is invalid"))?;
        if value.schema_version != SCHEMA
            || value.owner_id.is_empty()
            || value.owner_version.is_empty()
            || value.context_ref.is_empty()
            || value.key_reference.is_empty()
            || value.limits.max_items == 0
            || value.limits.max_context_bytes == 0
            || value.limits.max_objective_bytes == 0
            || value.limits.max_control_events == 0
            || value.limits.max_items > 64
            || value.limits.max_notes > 16
            || value.limits.max_context_bytes > 128 * 1024
            || value.limits.max_objective_bytes > 512
            || value.limits.max_control_events > 4096
        {
            return Err(String::from(
                "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is invalid",
            ));
        }
        Ok(value)
    }

    fn descriptor(&self) -> Result<ContextBindingDescriptor, ManagementError> {
        ContextBindingDescriptor {
            schema_version: sts2_harness::management::CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
            binding_id: format!("{}.decide.v1", self.configuration.owner_id),
            version: 1,
            digest: String::new(),
            context_ref: self.configuration.context_ref.clone(),
            node_kinds: vec!["decide".into()],
            sources: Vec::new(),
            operations: vec![
                ContextBindingOperation::Pause,
                ContextBindingOperation::Commit,
                ContextBindingOperation::Resume,
            ],
            effective_limits: self.configuration.limits.clone(),
            continuity: ContextBindingContinuity {
                survives_controller_restart: true,
                receipt_recovery: false,
                provider_session_continuity: false,
            },
            grants: ContextBindingGrants {
                metadata_read: true,
                content_read: false,
                edit: false,
                control: true,
            },
            state: ContextBindingState::Available,
        }
        .seal()
    }
}

impl ContextOwnerPort for Owner {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        ContextBindingCatalog {
            schema_version: sts2_harness::management::CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.into(),
            owner_id: self.configuration.owner_id.clone(),
            owner_version: self.configuration.owner_version.clone(),
            catalog_digest: String::new(),
            descriptors: vec![self.descriptor()?],
        }
        .seal()
    }
    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let descriptor = self.descriptor()?;
        if request.context_ref != descriptor.context_ref
            || request.node_kind != "decide"
            || request.binding_id != descriptor.binding_id
            || request.binding_digest != descriptor.digest
        {
            return Err(ManagementError::conflict(
                "context_owner_binding_stale",
                "context binding is not current",
            ));
        }
        let current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get(&request.workflow_run_id).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_observation_missing",
                "current runtime observation is unavailable",
            )
        })?;
        if entry.catalog_generation != Some(entry.authority.state().boundary.generation) {
            return Err(ManagementError::unavailable(
                "context_owner_catalog_missing",
                "current runtime legal-action catalog is unavailable",
            ));
        }
        if entry.actor != actor.subject {
            return Err(ManagementError::forbidden(
                "context_owner_actor",
                "actor cannot bind this context authority",
            ));
        }
        let state = entry.authority.state();
        Ok(ContextOwnerBinding {
            schema_version: sts2_harness::management::CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
            owner_id: self.configuration.owner_id.clone(),
            owner_version: self.configuration.owner_version.clone(),
            invocation_id: format!("{}.{}", request.workflow_run_id, request.node_execution_id),
            binding_id: descriptor.binding_id,
            binding_version: descriptor.version,
            binding_digest: descriptor.digest,
            context_ref: request.context_ref.clone(),
            instance_id: request.instance_id.clone(),
            node_kind: request.node_kind.clone(),
            state: ContextBindingState::Available,
            workflow_run_id: request.workflow_run_id.clone(),
            definition_digest: request.definition_digest.clone(),
            graph_id: request.graph_id.clone(),
            node_id: request.node_id.clone(),
            node_execution_id: request.node_execution_id.clone(),
            boundary: state.boundary.clone(),
            lease_epoch: state.boundary.controller_epoch,
            snapshot_id: format!("snapshot.{}", state.boundary.generation),
            approved_revision_id: state.active_revision_id.clone(),
            plan_epoch: state.plan_epoch,
            grants: descriptor.grants,
            continuity: descriptor.continuity,
        })
    }

    fn control(
        &self,
        actor: &AuthContext,
        binding: &ContextOwnerBinding,
        command: &sts2_harness::management::ContextControlCommand,
    ) -> Result<sts2_harness::management::ContextControlReceipt, ManagementError> {
        if !actor.can("workflow:control") || !actor.can_run(&binding.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "context_owner_control_forbidden",
                "actor cannot control this context authority",
            ));
        }
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&binding.workflow_run_id).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_association_unavailable",
                "current authority is unavailable",
            )
        })?;
        if entry.actor != actor.subject || entry.authority.state().boundary != binding.boundary {
            return Err(ManagementError::conflict(
                "context_owner_control_stale",
                "context control binding is stale",
            ));
        }
        let receipt = match command {
            sts2_harness::management::ContextControlCommand::Pause {
                idempotency_key,
                expected_control_version,
            } => entry
                .authority
                .request_pause(idempotency_key, *expected_control_version),
            sts2_harness::management::ContextControlCommand::Commit {
                idempotency_key,
                expected_control_version,
                expected_revision_id,
                expected_boundary,
                preview_manifest_digest,
                approved_manifest_digest,
            } => entry.authority.commit(
                idempotency_key,
                *expected_control_version,
                expected_revision_id,
                expected_boundary,
                preview_manifest_digest,
                approved_manifest_digest,
            ),
            sts2_harness::management::ContextControlCommand::Resume {
                idempotency_key,
                expected_control_version,
                expected_boundary,
            } => entry.authority.resume(
                idempotency_key,
                *expected_control_version,
                expected_boundary,
            ),
        }
        .map_err(|error| ManagementError::conflict("context_owner_control_refused", error))?;
        entry
            .store
            .persist(&entry.authority, StoreMode::Enabled)
            .map_err(|error| {
                ManagementError::unavailable("context_owner_persist", error.to_string())
            })?;
        let state = entry.authority.state();
        let kind = match command {
            sts2_harness::management::ContextControlCommand::Pause { .. } => {
                sts2_harness::management::ContextControlCommandKind::Pause
            }
            sts2_harness::management::ContextControlCommand::Commit { .. } => {
                sts2_harness::management::ContextControlCommandKind::Commit
            }
            sts2_harness::management::ContextControlCommand::Resume { .. } => {
                sts2_harness::management::ContextControlCommandKind::Resume
            }
        };
        Ok(sts2_harness::management::ContextControlReceipt {
            schema_version: sts2_harness::management::CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.into(),
            owner_id: self.configuration.owner_id.clone(),
            invocation_id: binding.invocation_id.clone(),
            binding_id: binding.binding_id.clone(),
            binding_digest: binding.binding_digest.clone(),
            command: kind,
            command_id: receipt.command_id,
            idempotency_key: receipt.idempotency_key,
            effect: receipt.effect,
            control_version: receipt.control_version,
            plan_epoch: receipt.plan_epoch,
            controller_epoch: state.boundary.controller_epoch,
            gate_epoch: state.boundary.gate_epoch,
            boundary: state.boundary.clone(),
            revision_id: None,
            preview_manifest_digest: None,
            approved_manifest_digest: None,
        })
    }
}
