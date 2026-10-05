// SPDX-License-Identifier: MIT

impl Owner {
    pub(super) fn create_preview_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerPreviewRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        validate_preview_request(request)?;
        let payload_digest = request_digest(request)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        let (mut state, record_version) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        if let Some(receipt) = self.existing_receipt(&state, actor, &request.request_id, &payload_digest)? {
            return Ok(receipt);
        }
        let binding = self.authorize_current_binding(actor, snapshot, entry, false)?;
        // Preview returns bounded metadata only. The owner keeps rendered bytes
        // private, so the caller needs the edit grant but not content-read.
        self.verify_edit_grant(actor, &binding, false)?;
        if request.expected_boundary != binding.boundary {
            return Err(stale_boundary());
        }
        let stored = state
            .drafts
            .get(&request.draft_id)
            .cloned()
            .filter(|draft| draft.envelope.actor_subject == actor.subject)
            .ok_or_else(|| ManagementError::invalid("context_draft_not_found", "owner draft was not found"))?;
        if stored.envelope.binding != binding
            || stored.envelope.draft.version != request.expected_version
            || stored.envelope.draft.base_revision_id != entry.authority.state().active_revision_id
        {
            return Err(ManagementError::conflict("context_draft_stale", "draft revision or full owner binding changed"));
        }
        let now = unix_time()?;
        let trusted = entry.trusted_render.as_ref().filter(|render| {
            render.actor_subject == actor.subject
                && render.binding == binding
                && render.runtime_lease_id == entry.runtime_lease_id
                && render.binding.boundary == entry.authority.state().boundary
                && render.config.revision == sts2_harness::EXO_SOURCE_REVISION
                && now.checked_sub(render.captured_at)
                    .is_some_and(|age| age <= PREVIEW_TTL_SECONDS)
        }).cloned().ok_or_else(|| ManagementError::unavailable(
            "context_owner_preview_unavailable",
            "trusted current host-thread render input and admitted provider configuration are unavailable",
        ))?;
        if provider_config_digest(&trusted.config)? != trusted.provider_config_digest
            || trusted.binding.boundary.configuration_sha256 != binding.boundary.configuration_sha256
        {
            return Err(ManagementError::conflict(
                "context_owner_preview_config_stale",
                "captured provider configuration no longer matches the current runtime boundary",
            ));
        }
        let active = self.verify_active_source(entry, &stored)?;
        if stored.envelope.retention_expires_at.is_some_and(|expires| expires <= now) {
            return Err(ManagementError::conflict("context_owner_draft_expired", "draft retention horizon has expired"));
        }
        let registry = self.eligible_registry(entry, &state, Some(&stored))?
            .into_iter()
            .map(|(key, value)| (key, value.item))
            .collect::<BTreeMap<_, _>>();
        let limits = ContextOwnerEffectiveLimitsView::compose(&self.catalog(actor)?, &binding)?;
        let prepared = limits.prepare_managed_render(sts2_harness::management::ContextOwnerRenderRequest {
            boundary: &binding.boundary,
            request: trusted.request,
            draft: &stored.envelope.draft,
            registry: &registry,
            config: &trusted.config,
            now,
            invocation_id: &binding.invocation_id,
            membership: self.configuration.membership.as_ref(),
            continuity: sts2_harness::context_control::MembershipContinuity::from_provider_session_continuity(
                binding.continuity.provider_session_continuity,
            ),
        }).map_err(|error| ManagementError::conflict("context_owner_preview_refused", error.to_string()))?;
        let source_expiry = active.as_ref().and_then(|(_, _, until)| (*until != u64::MAX).then_some(*until));
        let expires_at = now.checked_add(PREVIEW_TTL_SECONDS).ok_or_else(owner_capacity)?
            .min(source_expiry.unwrap_or(u64::MAX))
            .min(stored.envelope.retention_expires_at.unwrap_or(u64::MAX));
        if expires_at <= now { return Err(ManagementError::conflict("context_owner_preview_expired", "preview source validity has expired")); }
        if state.previews.len() >= MAX_PREVIEWS || state.revisions.len() >= MAX_REVISIONS {
            return Err(owner_capacity());
        }
        let preview_id = format!("preview.{:016x}", state.next_preview);
        state.next_preview = state.next_preview.checked_add(1).ok_or_else(owner_capacity)?;
        let preview = ContextOwnerPreviewEnvelope {
            schema_version: CONTEXT_OWNER_PREVIEW_SCHEMA_VERSION.to_owned(),
            preview_id,
            actor_subject: actor.subject.clone(),
            binding: binding.clone(),
            provider_config_digest: trusted.provider_config_digest,
            draft_id: request.draft_id.clone(),
            draft_version: stored.envelope.draft.version,
            base_revision_id: stored.envelope.draft.base_revision_id.clone(),
            manifest_digest: prepared.manifest_sha256,
            effect_class: "owner_local_preview_only".to_owned(),
            blockers: Vec::new(),
            created_at: now,
            expires_at,
        };
        state.previews.insert(preview.preview_id.clone(), preview.clone());
        let receipt = self.new_receipt(
            actor,
            &snapshot.workflow_run_id,
            &binding,
            "create_preview",
            request.request_id.clone(),
            payload_digest,
            ContextOwnerMutationResult::Preview(preview),
            now,
        );
        self.record_receipt(&mut state, receipt.clone())?;
        self.persist_draft_state(entry, &state, record_version)?;
        Ok(receipt)
    }

    pub(super) fn get_preview_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        preview_id: &str,
    ) -> Result<ContextOwnerPreviewEnvelope, ManagementError> {
        validate_identifier("context_preview_id", preview_id)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        self.authorize_current_binding(actor, snapshot, entry, true)?;
        let (state, _) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        let preview = state.previews.get(preview_id)
            .filter(|preview| preview.actor_subject == actor.subject)
            .cloned()
            .ok_or_else(|| ManagementError::invalid("context_preview_not_found", "owner preview was not found"))?;
        if preview.expires_at <= unix_time()? {
            return Err(ManagementError::conflict("context_preview_expired", "owner preview is no longer current"));
        }
        Ok(preview)
    }
}
