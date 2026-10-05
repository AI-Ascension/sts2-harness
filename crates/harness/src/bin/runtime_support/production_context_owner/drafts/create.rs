// SPDX-License-Identifier: MIT

impl Owner {
    pub(super) fn create_draft_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerDraftCreateRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        validate_create_request(request)?;
        let payload_digest = request_digest(request)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        let (mut state, record_version) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        if let Some(receipt) = self.existing_receipt(&state, actor, &request.request_id, &payload_digest)? {
            return Ok(receipt);
        }
        let binding = self.authorize_current_binding(actor, snapshot, entry, false)?;
        self.verify_edit_grant(actor, &binding, false)?;
        if request.expected_boundary != binding.boundary {
            return Err(stale_boundary());
        }
        let active_revision = entry.authority.state().active_revision_id.clone();
        if request.base_revision_id != active_revision {
            return Err(ManagementError::conflict(
                "context_draft_base_stale",
                "draft base revision is not the current owner revision",
            ));
        }
        if state.drafts.contains_key(&request.draft_id) {
            return Err(ManagementError::conflict(
                "context_draft_exists",
                "draft identity is already in use for this workflow run",
            ));
        }
        if state.drafts.len() >= MAX_DRAFTS || state.revisions.len() >= MAX_REVISIONS {
            return Err(owner_capacity());
        }
        let mut draft = ContextDraft::new(&request.draft_id, &active_revision);
        let active_source = entry
            .store
            .active_context_source(&active_revision)
            .map_err(store_error)?;
        let (base_source, retention_expires_at) = if let Some((source, snapshot)) = active_source {
            let advertised = self.advertised_source(&source.source_id)?;
            if advertised.version != source.version
                || advertised.digest != source.digest
                || validate_document(&snapshot.document)? != advertised.digest
            {
                return Err(ManagementError::conflict(
                    "context_draft_source_stale",
                    "active source does not match its immutable owner advertisement",
                ));
            }
            let valid_until = source_valid_until(&snapshot.document);
            draft = snapshot.document.draft;
            draft.draft_id = request.draft_id.clone();
            draft.version = 1;
            draft.base_revision_id = active_revision;
            draft.author_ref = actor.subject.clone();
            let expiry = (valid_until != u64::MAX).then_some(valid_until);
            (
                Some(SourceIdentity {
                    source_id: source.source_id,
                    version: source.version,
                    digest: source.digest,
                }),
                expiry,
            )
        } else {
            (None, None)
        };
        draft.author_ref = actor.subject.clone();
        let now = unix_time()?;
        let envelope = ContextOwnerDraftEnvelope {
            schema_version: CONTEXT_OWNER_DRAFT_SCHEMA_VERSION.to_owned(),
            actor_subject: actor.subject.clone(),
            binding: binding.clone(),
            created_at: now,
            updated_at: now,
            retention_expires_at,
            draft,
        };
        let revision = self.new_revision(&mut state, actor, &binding, &envelope.draft, now, retention_expires_at)?;
        let revision_id = revision.revision_id.clone();
        state.revisions.insert(revision_id, revision);
        let stored = StoredDraft {
            envelope: envelope.clone(),
            base_source,
            authored_item_keys: Vec::new(),
        };
        state.drafts.insert(request.draft_id.clone(), stored);
        let receipt = self.new_receipt(
            actor,
            &snapshot.workflow_run_id,
            &binding,
            "create_draft",
            request.request_id.clone(),
            payload_digest,
            ContextOwnerMutationResult::Draft(envelope),
            now,
        );
        self.record_receipt(&mut state, receipt.clone())?;
        self.persist_draft_state(entry, &state, record_version)?;
        Ok(receipt)
    }
}
