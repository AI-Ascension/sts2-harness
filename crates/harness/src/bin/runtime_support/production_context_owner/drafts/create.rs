// SPDX-License-Identifier: MIT

impl Owner {
    pub(super) fn create_draft_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerDraftCreateRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        self.create_draft_current_with_clock(actor, snapshot, request, unix_time)
    }

    fn create_draft_current_with_clock<F>(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerDraftCreateRequest,
        clock: F,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError>
    where
        F: FnOnce() -> Result<u64, ManagementError>,
    {
        validate_create_request(request)?;
        let payload_digest = request_digest(request)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current
            .get_mut(&snapshot.workflow_run_id)
            .ok_or_else(owner_unavailable)?;
        let (mut state, record_version) =
            self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        if let Some(receipt) =
            self.existing_receipt(&state, actor, &request.request_id, &payload_digest)?
        {
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
        let (base_source, retention_expires_at, now) =
            if let Some((active, snapshot)) = active_source {
                let (document, valid_until, now) =
                    if super::publication_active::is_publication_source(&active.source_id) {
                        let now = clock()?;
                        let (publication, source) =
                            self.load_active_publication(entry, &actor.subject, &active, now)?;
                        if !super::publication_active::publication_binding_matches(
                            &publication.receipt.binding,
                            &binding,
                        ) {
                            return Err(ManagementError::conflict(
                                "context_draft_source_stale",
                                "active publication belongs to a different owner binding",
                            ));
                        }
                        let valid_until = source_valid_until(&source.document)
                            .min(publication.receipt.expires_at);
                        (source.document, valid_until, now)
                    } else {
                        let advertised = self.advertised_source(&active.source_id)?;
                        if advertised.version != active.version
                            || advertised.digest != active.digest
                            || validate_document(&snapshot.document)? != advertised.digest
                        {
                            return Err(ManagementError::conflict(
                                "context_draft_source_stale",
                                "active source does not match its immutable owner advertisement",
                            ));
                        }
                        let valid_until = source_valid_until(&snapshot.document);
                        (snapshot.document, valid_until, clock()?)
                    };
                draft = document.draft;
                draft.draft_id = request.draft_id.clone();
                draft.version = 1;
                draft.base_revision_id = active_revision;
                draft.author_ref = actor.subject.clone();
                let expiry = (valid_until != u64::MAX).then_some(valid_until);
                (
                    Some(SourceIdentity {
                        source_id: active.source_id,
                        version: active.version,
                        digest: active.digest,
                    }),
                    expiry,
                    now,
                )
            } else {
                (None, None, clock()?)
            };
        draft.author_ref = actor.subject.clone();
        let envelope = ContextOwnerDraftEnvelope {
            schema_version: CONTEXT_OWNER_DRAFT_SCHEMA_VERSION.to_owned(),
            actor_subject: actor.subject.clone(),
            binding: binding.clone(),
            created_at: now,
            updated_at: now,
            retention_expires_at,
            draft,
        };
        let revision = self.new_revision(
            &mut state,
            actor,
            &binding,
            &envelope.draft,
            now,
            retention_expires_at,
        )?;
        let revision_id = revision.revision_id.clone();
        state.revisions.insert(revision_id, revision);
        let stored = StoredDraft {
            envelope: envelope.clone(),
            base_source,
            authored_item_keys: Vec::new(),
        };
        state.drafts.insert(request.draft_id.clone(), stored);
        let receipt = self.new_receipt(MutationReceiptInput {
            actor,
            workflow_run_id: &snapshot.workflow_run_id,
            binding: &binding,
            operation: "create_draft",
            request_id: request.request_id.clone(),
            payload_digest,
            result: ContextOwnerMutationResult::Draft(envelope),
            now,
        });
        self.record_receipt(&mut state, receipt.clone())?;
        self.persist_draft_state(entry, &state, record_version)?;
        Ok(receipt)
    }
}
