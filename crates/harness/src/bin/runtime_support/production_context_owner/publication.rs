// SPDX-License-Identifier: MIT

use super::publication_active::{publication_store_error, publication_unavailable};
use super::*;
use sts2_harness::context_control::{
    DurableContextOwnerPublicationWrite, DurableContextSourceSnapshot,
};
use sts2_harness::management::{
    CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION, CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION,
    CONTEXT_OWNER_PUBLISHED_SOURCES_VIEW_SCHEMA_VERSION, ContextOwnerDraftPublicationLookupRequest,
    ContextOwnerDraftPublicationReceipt, ContextOwnerDraftPublicationRequest,
    ContextOwnerPublishedSourcesView,
};

impl Owner {
    pub(super) fn publish_draft_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerDraftPublicationRequest,
    ) -> Result<ContextOwnerDraftPublicationReceipt, ManagementError> {
        self.publish_draft_current_at(actor, snapshot, request, unix_time)
    }

    pub(super) fn publish_draft_current_at<F>(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerDraftPublicationRequest,
        clock: F,
    ) -> Result<ContextOwnerDraftPublicationReceipt, ManagementError>
    where
        F: FnOnce() -> Result<u64, ManagementError>,
    {
        if !self.configuration.render_required {
            return Err(publication_unavailable());
        }
        if !actor.can("workflow:content:write")
            || !actor.can_run(&snapshot.workflow_run_id)
            || request.schema_version
                != sts2_harness::management::CONTEXT_OWNER_PUBLICATION_REQUEST_SCHEMA_VERSION
        {
            return Err(ManagementError::forbidden(
                "context_publication_forbidden",
                "actor cannot publish owner context for this workflow run",
            ));
        }
        let run_id = self.validate_snapshot_owner(actor, snapshot)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&run_id).ok_or_else(owner_unavailable)?;

        // Authenticate and resolve exact replay before inspecting any live draft, binding,
        // boundary, expiry, or owner-state CAS precondition.
        let lookup = ContextOwnerDraftPublicationLookupRequest {
            schema_version: CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION.to_owned(),
            request: request.clone(),
        };
        if let Some(existing) = entry
            .store
            .recover_draft_publication(&self.configuration.owner_id, &actor.subject, &lookup)
            .map_err(publication_store_error)?
        {
            return Ok(existing.receipt);
        }

        let now = clock()?;
        let candidate = self.draft_publication_candidate(actor, snapshot, entry, request, now)?;
        let (source_id, request_digest) = entry
            .store
            .draft_publication_identity(&self.configuration.owner_id, &actor.subject, request)
            .map_err(publication_store_error)?;
        if self
            .configuration
            .sources
            .iter()
            .any(|source| source.source_id == source_id)
        {
            return Err(ManagementError::conflict(
                "context_publication_source_collision",
                "run-local publication identity conflicts with an owner catalog source",
            ));
        }
        let source_digest = validate_document(&candidate.document)?;
        let source = DurableContextSourceSnapshot {
            source_id: source_id.clone(),
            version: 1,
            digest: source_digest.clone(),
            document: candidate.document,
        };
        let resulting_owner_state_version = candidate
            .owner_state_version
            .checked_add(1)
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_publication_owner_version",
                    "owner-state version is outside its bound",
                )
            })?;
        let receipt = ContextOwnerDraftPublicationReceipt {
            schema_version: CONTEXT_OWNER_PUBLICATION_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            workflow_run_id: run_id,
            actor_subject: actor.subject.clone(),
            binding: candidate.binding,
            boundary: request.expected_boundary.clone(),
            request_id: request.request_id.clone(),
            request_digest,
            draft_id: request.draft_id.clone(),
            draft_version: candidate.draft.draft.version,
            base_revision_id: candidate.draft.draft.base_revision_id,
            expected_owner_state_version: candidate.owner_state_version,
            resulting_owner_state_version,
            source_id,
            source_version: 1,
            source_digest,
            published_at: now,
            expires_at: candidate.expires_at,
        };
        let publication = entry
            .store
            .publish_draft_source(DurableContextOwnerPublicationWrite {
                owner_id: &self.configuration.owner_id,
                actor_subject: &actor.subject,
                request,
                receipt: &receipt,
                source: &source,
                expected_owner_state_bytes: &candidate.owner_state_bytes,
                configured_source_count: self.configuration.sources.len(),
            })
            .map_err(publication_store_error)?;
        Ok(publication.receipt)
    }

    pub(super) fn recover_publication_receipt_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        lookup: &ContextOwnerDraftPublicationLookupRequest,
    ) -> Result<Option<ContextOwnerDraftPublicationReceipt>, ManagementError> {
        if !actor.can("workflow:read") || !actor.can_run(&snapshot.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "context_publication_recovery_forbidden",
                "actor cannot read publication history for this workflow run",
            ));
        }
        if lookup.schema_version != CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION {
            return Err(ManagementError::invalid(
                "context_publication_lookup_schema",
                "unsupported publication receipt lookup schema",
            ));
        }
        let run_id = self.validate_snapshot_owner(actor, snapshot)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&run_id).ok_or_else(owner_unavailable)?;
        entry
            .store
            .recover_draft_publication(&self.configuration.owner_id, &actor.subject, lookup)
            .map(|publication| publication.map(|publication| publication.receipt))
            .map_err(publication_store_error)
    }

    pub(super) fn published_sources_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerPublishedSourcesView, ManagementError> {
        if !self.configuration.render_required {
            return Err(publication_unavailable());
        }
        let run_id = self.validate_snapshot_owner(actor, snapshot)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&run_id).ok_or_else(owner_unavailable)?;
        let binding = self.authorize_current_binding(actor, snapshot, entry, true)?;
        let state = entry
            .store
            .load_owner_context_state(&self.configuration.owner_id)
            .map_err(publication_store_error)?;
        let owner_state_version = state.as_ref().map_or(0, |state| state.record_version);
        let publications = entry
            .store
            .list_draft_publications(&self.configuration.owner_id, &actor.subject)
            .map_err(publication_store_error)?
            .into_iter()
            .map(|publication| ContextBindingSource {
                source_id: publication.receipt.source_id,
                version: publication.receipt.source_version,
                digest: publication.receipt.source_digest,
            })
            .collect::<Vec<_>>();
        if publications.len() > sts2_harness::management::MAX_CONTEXT_SOURCES {
            return Err(ManagementError::unavailable(
                "context_publication_capacity",
                "published source metadata exceeds the bounded owner catalog",
            ));
        }
        let active_source = self.active_source_metadata(entry, actor, &binding)?;
        Ok(ContextOwnerPublishedSourcesView {
            schema_version: CONTEXT_OWNER_PUBLISHED_SOURCES_VIEW_SCHEMA_VERSION.to_owned(),
            owner_id: binding.owner_id.clone(),
            workflow_run_id: run_id,
            definition_digest: snapshot.definition_digest.clone(),
            instance_id: binding.instance_id.clone(),
            boundary: binding.boundary.clone(),
            binding,
            owner_state_version,
            active_source,
            publications,
        })
    }
}
