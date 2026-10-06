// SPDX-License-Identifier: MIT

use super::super::super::context_owner::ContextBindingSource;
use super::*;

impl ManagementService {
    pub fn context_owner_published_sources(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextOwnerPublishedSourcesView, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let view = self.context_owner.published_sources(actor, &snapshot)?;
        validate_published_sources_view(&snapshot, &view)?;
        Ok(view)
    }

    pub fn publish_context_owner_draft(
        &self,
        actor: &AuthContext,
        run_id: &str,
        path_draft_id: &str,
        request: &ContextOwnerDraftPublicationRequest,
    ) -> Result<ContextOwnerDraftPublicationReceipt, ManagementError> {
        validate_identifier("run_id", run_id)?;
        validate_identifier("context_draft_id", path_draft_id)?;
        authorize(actor, "workflow:content:write", Some(run_id))?;
        request.validate()?;
        if request.draft_id != path_draft_id {
            return Err(ManagementError::invalid(
                "context_publication_path_mismatch",
                "draft ID in the path must match the publication request",
            ));
        }
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        if request.expected_boundary.run_id != snapshot.workflow_run_id {
            return Err(ManagementError::conflict(
                "context_publication_request_scope",
                "publication request does not name the workflow run in the path",
            ));
        }
        let receipt = self
            .context_owner
            .publish_draft(actor, &snapshot, request)?;
        validate_publication_receipt(actor, &snapshot, request, &receipt)?;
        Ok(receipt)
    }

    pub fn recover_context_owner_publication_receipt(
        &self,
        actor: &AuthContext,
        run_id: &str,
        lookup: &ContextOwnerDraftPublicationLookupRequest,
    ) -> Result<Option<ContextOwnerDraftPublicationReceipt>, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        if lookup.schema_version != CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION {
            return Err(ManagementError::invalid(
                "context_publication_lookup_schema",
                "unsupported publication receipt lookup schema",
            ));
        }
        lookup.request.validate()?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        if lookup.request.expected_boundary.run_id != snapshot.workflow_run_id {
            return Err(ManagementError::conflict(
                "context_publication_lookup_scope",
                "publication lookup does not name the workflow run in the path",
            ));
        }
        let receipt = self
            .context_owner
            .recover_publication_receipt(actor, &snapshot, lookup)?;
        if let Some(receipt) = &receipt {
            validate_publication_receipt(actor, &snapshot, &lookup.request, receipt)?;
        }
        Ok(receipt)
    }
}

pub(super) fn validate_published_sources_view(
    snapshot: &RunSnapshot,
    view: &ContextOwnerPublishedSourcesView,
) -> Result<(), ManagementError> {
    if view.schema_version != CONTEXT_OWNER_PUBLISHED_SOURCES_VIEW_SCHEMA_VERSION
        || view.workflow_run_id != snapshot.workflow_run_id
        || view.definition_digest != snapshot.definition_digest
        || view.owner_id != view.binding.owner_id
        || view.instance_id != view.binding.instance_id
        || view.boundary != view.binding.boundary
        || view.publications.len() > crate::management::MAX_CONTEXT_SOURCES
    {
        return Err(ManagementError::conflict(
            "context_publication_view_scope",
            "published-source view does not match the admitted run",
        ));
    }
    view.binding.validate(Some(snapshot))?;
    if snapshot
        .admission
        .as_ref()
        .is_some_and(|admission| admission.target.instance_id != view.instance_id)
    {
        return Err(ManagementError::conflict(
            "context_publication_view_instance",
            "published-source view does not match the admitted target instance",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    for source in &view.publications {
        validate_source_metadata(source)?;
        if !identities.insert(source.source_id.as_str()) {
            return Err(ManagementError::conflict(
                "context_publication_view_duplicate",
                "published-source view contains duplicate source identities",
            ));
        }
    }
    if let Some(source) = &view.active_source {
        validate_source_metadata(source)?;
    }
    if view.owner_state_version == 0 && !view.publications.is_empty() {
        return Err(ManagementError::conflict(
            "context_publication_view_version",
            "published sources require a durable owner-state version",
        ));
    }
    Ok(())
}

fn validate_source_metadata(source: &ContextBindingSource) -> Result<(), ManagementError> {
    validate_identifier("context_source_id", &source.source_id)?;
    if source.version == 0 {
        return Err(ManagementError::invalid(
            "context_publication_source_version",
            "published source version must be positive",
        ));
    }
    super::super::super::contract::validate_digest("context_source_digest", &source.digest)
        .map_err(ManagementError::from)
}

fn validate_publication_receipt(
    actor: &AuthContext,
    snapshot: &RunSnapshot,
    request: &ContextOwnerDraftPublicationRequest,
    receipt: &ContextOwnerDraftPublicationReceipt,
) -> Result<(), ManagementError> {
    let request_digest = request.digest()?;
    let resulting_version = request
        .expected_owner_state_version
        .checked_add(1)
        .ok_or_else(|| {
            ManagementError::invalid(
                "context_publication_owner_version",
                "publication owner-state version is outside its bound",
            )
        })?;
    if receipt.schema_version != CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION
        || receipt.workflow_run_id != snapshot.workflow_run_id
        || receipt.actor_subject != actor.subject
        || receipt.request_id != request.request_id
        || receipt.request_digest != request_digest
        || receipt.draft_id != request.draft_id
        || receipt.draft_version != request.expected_draft_version
        || receipt.base_revision_id != request.expected_base_revision_id
        || receipt.expected_owner_state_version != request.expected_owner_state_version
        || receipt.resulting_owner_state_version != resulting_version
        || receipt.source_version != 1
        || receipt.boundary != request.expected_boundary
        || receipt.binding.owner_id != receipt.owner_id
        || receipt.binding.binding_id != request.expected_binding_id
        || receipt.binding.binding_digest != request.expected_binding_digest
        || receipt.expires_at <= receipt.published_at
        || receipt.expires_at == u64::MAX
    {
        return Err(ManagementError::conflict(
            "context_publication_receipt_mismatch",
            "owner returned a publication receipt outside the exact request scope",
        ));
    }
    validate_identifier("context_owner_id", &receipt.owner_id)?;
    validate_identifier("context_source_id", &receipt.source_id)?;
    super::super::super::contract::validate_digest("context_source_digest", &receipt.source_digest)
        .map_err(ManagementError::from)?;
    receipt.binding.validate(Some(snapshot))?;
    Ok(())
}
