// SPDX-License-Identifier: MIT

use super::*;

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
        if request.instance_id != entry.runtime_instance_id {
            return Err(ManagementError::conflict(
                "context_owner_instance",
                "context binding instance does not match the trusted runtime instance",
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
        if entry.trusted_render.as_ref().is_some_and(|render| {
            render.binding.invocation_id != binding.invocation_id
                || render.binding.binding_id != binding.binding_id
                || render.binding.binding_digest != binding.binding_digest
                || render.binding.boundary != binding.boundary
        }) {
            entry.trusted_render = None;
        }
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

    fn source_status(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerSourceStatus, ManagementError> {
        self.source_status_current(actor, snapshot)
    }

    fn publish_source(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        source_id: &str,
        document: &sts2_harness::context_control::ContextSourceDocument,
    ) -> Result<ContextBindingSource, ManagementError> {
        self.publish_source_current(actor, snapshot, source_id, document)
    }

    fn adopt_source(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        source_id: &str,
        request: &sts2_harness::management::ContextSourceAdoptionRequest,
    ) -> Result<sts2_harness::management::ContextControlReceipt, ManagementError> {
        self.adopt_source_current(actor, snapshot, source_id, request)
    }

    fn render_required(&self) -> bool {
        self.configuration.render_required
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

    fn eligible_items(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        draft_id: Option<&str>,
        include_content: bool,
    ) -> Result<sts2_harness::management::ContextOwnerItemsView, ManagementError> {
        self.eligible_items_current(actor, snapshot, draft_id, include_content)
    }

    fn list_drafts(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<sts2_harness::management::ContextOwnerDraftListView, ManagementError> {
        self.list_drafts_current(actor, snapshot)
    }

    fn get_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        draft_id: &str,
    ) -> Result<sts2_harness::management::ContextOwnerDraftEnvelope, ManagementError> {
        self.get_draft_current(actor, snapshot, draft_id)
    }

    fn create_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerDraftCreateRequest,
    ) -> Result<sts2_harness::management::ContextOwnerMutationReceipt, ManagementError> {
        self.create_draft_current(actor, snapshot, request)
    }

    fn patch_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerDraftPatchRequest,
    ) -> Result<sts2_harness::management::ContextOwnerMutationReceipt, ManagementError> {
        self.patch_draft_current(actor, snapshot, request)
    }

    fn list_revisions(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        after_revision_id: Option<&str>,
        limit: u64,
    ) -> Result<sts2_harness::management::ContextOwnerRevisionPage, ManagementError> {
        self.list_revisions_current(actor, snapshot, after_revision_id, limit)
    }

    fn get_revision(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        revision_id: &str,
    ) -> Result<sts2_harness::management::ContextOwnerRevisionEnvelope, ManagementError> {
        self.get_revision_current(actor, snapshot, revision_id)
    }

    fn create_preview(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerPreviewRequest,
    ) -> Result<sts2_harness::management::ContextOwnerMutationReceipt, ManagementError> {
        self.create_preview_current(actor, snapshot, request)
    }

    fn get_preview(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        preview_id: &str,
    ) -> Result<sts2_harness::management::ContextOwnerPreviewEnvelope, ManagementError> {
        self.get_preview_current(actor, snapshot, preview_id)
    }

    fn publish_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerDraftPublicationRequest,
    ) -> Result<sts2_harness::management::ContextOwnerDraftPublicationReceipt, ManagementError>
    {
        self.publish_draft_current(actor, snapshot, request)
    }

    fn published_sources(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<sts2_harness::management::ContextOwnerPublishedSourcesView, ManagementError> {
        self.published_sources_current(actor, snapshot)
    }

    fn recover_publication_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerDraftPublicationLookupRequest,
    ) -> Result<
        Option<sts2_harness::management::ContextOwnerDraftPublicationReceipt>,
        ManagementError,
    > {
        self.recover_publication_receipt_current(actor, snapshot, request)
    }

    fn recover_mutation_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerMutationRequest,
    ) -> Result<Option<sts2_harness::management::ContextOwnerMutationReceipt>, ManagementError>
    {
        self.recover_mutation_receipt_current(actor, snapshot, request)
    }
}
