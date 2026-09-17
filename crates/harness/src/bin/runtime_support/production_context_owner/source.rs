// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::{
    ContextSourceDocument, DurableActiveContextSource, DurableContextSourceSnapshot, StoreMode,
};
use sts2_harness::management::{
    CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, ContextControlCommand, ContextControlCommandKind,
    ContextSourceAdoptionRequest,
};

impl Owner {
    pub(super) fn publish_source_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        source_id: &str,
        document: &ContextSourceDocument,
    ) -> Result<ContextBindingSource, ManagementError> {
        if !self.configuration.render_required {
            return Err(ManagementError::unavailable(
                "context_source_publication_unavailable",
                "managed context rendering is not enabled for this owner",
            ));
        }
        let run = self.validate_snapshot_owner(actor, snapshot)?;
        let advertised = self.advertised_source(source_id)?;
        let digest = validate_document(document)?;
        if digest != advertised.digest {
            return Err(ManagementError::conflict(
                "context_source_not_advertised",
                "source bytes do not match the immutable identity advertised by this owner",
            ));
        }
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&run).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_source_status_unavailable",
                "fresh runtime observation is unavailable",
            )
        })?;
        self.validate_source_entry(actor, snapshot, entry)?;
        entry
            .store
            .publish_context_source(&DurableContextSourceSnapshot {
                source_id: advertised.source_id.clone(),
                version: advertised.version,
                digest: advertised.digest.clone(),
                document: document.clone(),
            })
            .map_err(|error| {
                let code = match error {
                    sts2_harness::context_control::DurableControlStoreError::SourceConflict => {
                        "context_source_conflict"
                    }
                    sts2_harness::context_control::DurableControlStoreError::TooLarge => {
                        "context_source_too_large"
                    }
                    _ => "context_source_persist",
                };
                ManagementError::unavailable(code, error.to_string())
            })?;
        Ok(advertised)
    }

    pub(super) fn adopt_source_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        source_id: &str,
        request: &ContextSourceAdoptionRequest,
    ) -> Result<sts2_harness::management::ContextControlReceipt, ManagementError> {
        if !self.configuration.render_required {
            return Err(ManagementError::unavailable(
                "context_source_adoption_unavailable",
                "managed context rendering is not enabled for this owner",
            ));
        }
        if !actor.can("workflow:control") || !actor.can_run(&snapshot.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "context_source_adoption_forbidden",
                "actor cannot adopt a context source for this workflow run",
            ));
        }
        let run = self.validate_snapshot_owner(actor, snapshot)?;
        let advertised = self.advertised_source(source_id)?;
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&run).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_source_status_unavailable",
                "fresh runtime observation is unavailable",
            )
        })?;
        self.validate_source_entry(actor, snapshot, entry)?;
        let command = ContextControlCommand::Commit {
            idempotency_key: request.idempotency_key.clone(),
            expected_control_version: request.expected_control_version,
            expected_revision_id: request.expected_revision_id.clone(),
            expected_boundary: request.expected_boundary.clone(),
            preview_manifest_digest: advertised.digest.clone(),
            approved_manifest_digest: advertised.digest.clone(),
        };
        if let Some(existing) = entry
            .store
            .lookup_owner_control_receipt(&self.configuration.owner_id, &actor.subject, &command)
            .map_err(|error| {
                ManagementError::unavailable("context_source_receipt_lookup", error.to_string())
            })?
        {
            existing.receipt.validate_for(&existing.binding, &command)?;
            return Ok(existing.receipt);
        }
        if entry.catalog_generation != Some(entry.authority.state().boundary.generation) {
            return Err(ManagementError::unavailable(
                "context_owner_source_status_unavailable",
                "fresh runtime legal-action catalog is unavailable",
            ));
        }
        let state = entry.authority.state().clone();
        if request.expected_revision_id != state.active_revision_id
            || request.expected_control_version != state.boundary.control_version
            || request.expected_boundary != state.boundary
        {
            return Err(ManagementError::conflict(
                "context_source_adoption_stale",
                "source adoption precondition is not the current owner control fence",
            ));
        }
        let source = entry
            .store
            .load_context_source(
                &advertised.source_id,
                advertised.version,
                &advertised.digest,
            )
            .map_err(|error| {
                ManagementError::unavailable("context_source_load", error.to_string())
            })?
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_source_not_published",
                    "the advertised source has not been published for this run",
                )
            })?;
        if source.document.draft.base_revision_id != state.active_revision_id {
            return Err(ManagementError::conflict(
                "context_source_base_revision",
                "published source was prepared from a different active revision",
            ));
        }

        let descriptor = catalog.descriptor_for(&self.configuration.context_ref, "decide")?;
        if !descriptor.grants.content_read
            || !descriptor
                .sources
                .iter()
                .any(|candidate| candidate == &advertised)
        {
            return Err(ManagementError::capability(
                "context_source_grant_missing",
                "current binding does not grant access to the advertised source",
            ));
        }
        self.validate_control_limits_in_catalog(&catalog, &entry.admitted_control_limits)?;
        let binding_request = entry.binding_request.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_association_unavailable",
                "source adoption requires an owner binding for the current workflow cursor",
            )
        })?;
        if binding_request.workflow_run_id != run
            || binding_request.definition_digest != snapshot.definition_digest
            || binding_request.instance_id != entry.runtime_instance_id
            || binding_request.graph_id != snapshot.cursor.graph_id
            || binding_request.node_id != snapshot.cursor.node_id
            || binding_request.node_execution_id != snapshot.cursor.node_execution_id
            || binding_request.node_kind != "decide"
            || binding_request.context_ref != self.configuration.context_ref
        {
            return Err(ManagementError::conflict(
                "context_source_binding_stale",
                "source adoption requires the current admitted decision binding",
            ));
        }
        let binding_request = binding_request.clone();
        let binding = self.binding_for_request(&binding_request, entry, &catalog)?;

        let mut authority = entry.authority.clone();
        let outcome = authority
            .adopt_source_revision(
                &request.idempotency_key,
                request.expected_control_version,
                &request.expected_revision_id,
                &request.expected_boundary,
                &advertised.source_id,
                advertised.version,
                &advertised.digest,
            )
            .map_err(|error| ManagementError::conflict("context_source_adoption_refused", error))?;
        let state = authority.state();
        let receipt = sts2_harness::management::ContextControlReceipt {
            schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            invocation_id: binding.invocation_id.clone(),
            binding_id: binding.binding_id.clone(),
            binding_digest: binding.binding_digest.clone(),
            command: ContextControlCommandKind::Commit,
            command_id: outcome.command_id,
            idempotency_key: outcome.idempotency_key,
            effect: outcome.effect,
            control_version: outcome.control_version,
            plan_epoch: outcome.plan_epoch,
            controller_epoch: state.boundary.controller_epoch,
            gate_epoch: state.boundary.gate_epoch,
            boundary: state.boundary.clone(),
            revision_id: Some(state.active_revision_id.clone()),
            preview_manifest_digest: Some(advertised.digest.clone()),
            approved_manifest_digest: Some(advertised.digest.clone()),
        };
        receipt.validate_for(&binding, &command)?;
        let durable = sts2_harness::context_control::DurableContextOwnerControlReceipt {
            owner_id: self.configuration.owner_id.clone(),
            actor_subject: actor.subject.clone(),
            binding,
            command,
            receipt: receipt.clone(),
        };
        let activation = DurableActiveContextSource {
            source_id: advertised.source_id,
            version: advertised.version,
            digest: advertised.digest,
            active_revision_id: state.active_revision_id.clone(),
        };
        entry
            .store
            .persist_with_owner_control_receipt_and_source(
                &authority,
                StoreMode::Enabled,
                &durable,
                &activation,
            )
            .map_err(|error| {
                ManagementError::unavailable("context_source_adoption_persist", error.to_string())
            })?;
        entry.authority = authority;
        Ok(receipt)
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;

#[path = "source_render.rs"]
mod source_render;
#[path = "source_validation.rs"]
mod source_validation;
use source_validation::{source_valid_until, unix_time, validate_document};
