// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::StoreMode;

#[path = "binding_association.rs"]
mod association;
#[path = "binding_control.rs"]
mod control;

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
                receipt_recovery: true,
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

    fn binding_for_request(
        &self,
        request: &ContextBindingRequest,
        entry: &Current,
        catalog: &ContextBindingCatalog,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let descriptor = self.descriptor()?;
        let state = entry.authority.state();
        let binding = ContextOwnerBinding {
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
            lease_epoch: entry.runtime_lease_epoch,
            snapshot_id: format!("snapshot.{}", state.boundary.generation),
            approved_revision_id: state.active_revision_id.clone(),
            plan_epoch: state.plan_epoch,
            grants: descriptor.grants,
            continuity: descriptor.continuity,
        };
        ContextOwnerEffectiveLimitsView::compose(catalog, &binding)?;
        Ok(binding)
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
        request.validate()?;
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
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
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&request.workflow_run_id).ok_or_else(|| {
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
        self.validate_control_limits_in_catalog(&catalog, &entry.admitted_control_limits)?;
        let binding = self.binding_for_request(request, entry, &catalog)?;
        let selected_limits = &entry.admitted_control_limits;
        // This owner publishes `configuration.limits` in its descriptor, so
        // the catalog check above also bounds the composed current binding.
        ContextOwnerEffectiveLimitsView::compose(&catalog, &binding)?;
        let authority = entry
            .authority
            .clone()
            .with_max_control_events(selected_limits.max_control_events)
            .map_err(|code| {
                let reason = if code == "context_control_events_exhausted" {
                    "context_control_events_exhausted"
                } else {
                    "context_control_event_limit_invalid"
                };
                ManagementError::conflict(
                    reason,
                    "current context control authority exceeds the admitted run limit",
                )
            })?;
        entry.authority = authority;
        entry
            .store
            .persist(&entry.authority, StoreMode::Enabled)
            .map_err(|error| {
                ManagementError::unavailable("context_owner_persist", error.to_string())
            })?;
        entry.binding_request = Some(request.clone());
        Ok(binding)
    }

    fn association(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        self.current_association(actor, snapshot)
    }

    fn control(
        &self,
        actor: &AuthContext,
        binding: &ContextOwnerBinding,
        command: &sts2_harness::management::ContextControlCommand,
    ) -> Result<sts2_harness::management::ContextControlReceipt, ManagementError> {
        self.control_current(actor, binding, command)
    }

    fn recover_control_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        command: &sts2_harness::management::ContextControlCommand,
    ) -> Result<Option<sts2_harness::management::ContextControlReceiptRecovery>, ManagementError>
    {
        self.recover_historical_receipt(actor, snapshot, command)
    }
}
