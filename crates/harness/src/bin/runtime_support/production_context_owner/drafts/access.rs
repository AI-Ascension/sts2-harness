// SPDX-License-Identifier: MIT

impl Owner {
    pub(super) fn eligible_items_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        draft_id: Option<&str>,
        include_content: bool,
    ) -> Result<ContextOwnerItemsView, ManagementError> {
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        let binding = self.authorize_current_binding(actor, snapshot, entry, true)?;
        if include_content
            && (!actor.can("workflow:context:content:read") || !binding.grants.content_read)
        {
            return Err(ManagementError::forbidden(
                "context_owner_content_forbidden",
                "current actor and binding must both grant bounded context content access",
            ));
        }
        let (state, _) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        let draft = draft_id
            .map(|draft_id| {
                validate_identifier("context_draft_id", draft_id)?;
                state
                    .drafts
                    .get(draft_id)
                    .filter(|stored| stored.envelope.actor_subject == actor.subject)
                    .ok_or_else(|| {
                        ManagementError::invalid(
                            "context_owner_draft_not_found",
                            "owner draft was not found",
                        )
                    })
            })
            .transpose()?;
        if let Some(draft) = draft {
            if draft.envelope.binding != binding
                || draft.envelope.draft.base_revision_id
                    != entry.authority.state().active_revision_id
            {
                return Err(ManagementError::conflict(
                    "context_draft_stale",
                    "eligible items require the exact current draft binding and base revision",
                ));
            }
            let active = self.verify_active_source(entry, draft)?;
            if !draft.authored_item_keys.is_empty() {
                let Some((_, _, valid_until)) = active else {
                    return Err(ManagementError::conflict(
                        "context_owner_authored_retention_unavailable",
                        "authored items require the draft's exact active source",
                    ));
                };
                let now = unix_time()?;
                if valid_until == u64::MAX
                    || valid_until <= now
                    || draft.envelope.retention_expires_at != Some(valid_until)
                {
                    return Err(ManagementError::conflict(
                        "context_owner_authored_retention_expired",
                        "authored items are outside their original finite source horizon",
                    ));
                }
            }
        }
        let registry = self.eligible_registry(entry, &state, draft)?;
        let now = unix_time()?;
        let items = registry
            .into_values()
            .filter(|eligible| eligible.item.expires_at > now)
            .map(|eligible| ContextOwnerItemView {
                reference: eligible.item.reference,
                kind: eligible.item.kind,
                byte_length: eligible.item.bytes.len() as u64,
                protected: eligible.item.protected,
                expires_at: eligible.item.expires_at,
                source_id: eligible.source.source_id,
                source_version: eligible.source.version,
                source_digest: eligible.source.digest,
                content: include_content.then_some(eligible.item.bytes),
            })
            .collect();
        Ok(ContextOwnerItemsView {
            schema_version: CONTEXT_OWNER_ITEMS_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            workflow_run_id: snapshot.workflow_run_id.clone(),
            binding_id: binding.binding_id,
            binding_digest: binding.binding_digest,
            boundary: binding.boundary,
            items,
        })
    }

    pub(super) fn list_drafts_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerDraftListView, ManagementError> {
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        self.authorize_current_binding(actor, snapshot, entry, true)?;
        let (state, _) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        Ok(ContextOwnerDraftListView {
            schema_version: CONTEXT_OWNER_DRAFT_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            workflow_run_id: snapshot.workflow_run_id.clone(),
            drafts: state
                .drafts
                .values()
                .filter(|draft| draft.envelope.actor_subject == actor.subject)
                .map(|draft| draft.envelope.clone())
                .collect(),
        })
    }

    pub(super) fn get_draft_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        draft_id: &str,
    ) -> Result<ContextOwnerDraftEnvelope, ManagementError> {
        validate_identifier("context_draft_id", draft_id)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        self.authorize_current_binding(actor, snapshot, entry, true)?;
        let (state, _) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        state
            .drafts
            .get(draft_id)
            .filter(|draft| draft.envelope.actor_subject == actor.subject)
            .map(|draft| draft.envelope.clone())
            .ok_or_else(|| ManagementError::invalid("context_draft_not_found", "owner draft was not found"))
    }

    pub(super) fn list_revisions_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        after_revision_id: Option<&str>,
        limit: u64,
    ) -> Result<ContextOwnerRevisionPage, ManagementError> {
        if !(1..=MAX_CONTEXT_OWNER_PAGE_SIZE).contains(&limit) {
            return Err(ManagementError::invalid("context_revision_page_limit", "revision page limit is outside the supported bound"));
        }
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        self.authorize_current_binding(actor, snapshot, entry, true)?;
        let (state, _) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        let mut revisions = state.revisions.values()
            .filter(|revision| revision.actor_subject == actor.subject)
            .filter(|revision| after_revision_id.is_none_or(|after| revision.revision_id.as_str() > after))
            .take(limit as usize + 1)
            .cloned()
            .collect::<Vec<_>>();
        let has_more = revisions.len() > limit as usize;
        if has_more { revisions.pop(); }
        let next_after_revision_id = has_more.then(|| revisions.last().map(|revision| revision.revision_id.clone())).flatten();
        Ok(ContextOwnerRevisionPage {
            schema_version: CONTEXT_OWNER_REVISION_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            workflow_run_id: snapshot.workflow_run_id.clone(),
            revisions,
            next_after_revision_id,
        })
    }

    pub(super) fn get_revision_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        revision_id: &str,
    ) -> Result<ContextOwnerRevisionEnvelope, ManagementError> {
        validate_identifier("context_revision_id", revision_id)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(owner_unavailable)?;
        self.authorize_current_binding(actor, snapshot, entry, true)?;
        let (state, _) = self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        state.revisions.get(revision_id)
            .filter(|revision| revision.actor_subject == actor.subject)
            .cloned()
            .ok_or_else(|| ManagementError::invalid("context_revision_not_found", "owner revision was not found"))
    }
}
