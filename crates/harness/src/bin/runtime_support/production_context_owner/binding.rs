// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::StoreMode;

#[path = "binding_association.rs"]
mod association;
#[path = "binding_control.rs"]
mod control;
#[path = "binding_port.rs"]
mod port;

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
            || value.sources.len() > 16
            || (value.render_required && value.sources.is_empty())
        {
            return Err(String::from(
                "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG is invalid",
            ));
        }
        let mut source_ids = std::collections::BTreeSet::new();
        for source in &value.sources {
            if source.source_id.is_empty()
                || source.source_id.len() > 128
                || !source.source_id.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
                })
                || source.version == 0
                || source.digest.len() != 64
                || !source
                    .digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                || !source_ids.insert(source.source_id.as_str())
            {
                return Err(String::from(
                    "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG contains an invalid source",
                ));
            }
        }
        Ok(value)
    }

    fn descriptor(&self) -> Result<ContextBindingDescriptor, ManagementError> {
        let mut operations = vec![
            ContextBindingOperation::Pause,
            ContextBindingOperation::Commit,
            ContextBindingOperation::Resume,
        ];
        if self.configuration.render_required {
            operations.extend([
                ContextBindingOperation::IncludeItem,
                ContextBindingOperation::ExcludeItem,
                ContextBindingOperation::PinItem,
                ContextBindingOperation::UnpinItem,
                ContextBindingOperation::PutNote,
                ContextBindingOperation::RemoveNote,
                ContextBindingOperation::SetObjective,
            ]);
        }
        ContextBindingDescriptor {
            schema_version: sts2_harness::management::CONTEXT_OWNER_BINDING_SCHEMA_VERSION.into(),
            binding_id: format!("{}.decide.v1", self.configuration.owner_id),
            version: 1,
            digest: String::new(),
            context_ref: self.configuration.context_ref.clone(),
            node_kinds: vec!["decide".into()],
            sources: self.configuration.sources.clone(),
            operations,
            effective_limits: self.configuration.limits.clone(),
            continuity: ContextBindingContinuity {
                survives_controller_restart: true,
                receipt_recovery: true,
                provider_session_continuity: false,
            },
            grants: ContextBindingGrants {
                metadata_read: true,
                content_read: self.configuration.render_required,
                edit: self.configuration.render_required,
                control: true,
            },
            state: ContextBindingState::Available,
        }
        .seal()
    }

    pub(super) fn binding_for_request(
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
