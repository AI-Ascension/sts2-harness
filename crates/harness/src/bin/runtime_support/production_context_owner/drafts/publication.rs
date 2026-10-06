// SPDX-License-Identifier: MIT

pub(super) struct DraftPublicationCandidate {
    pub(super) binding: ContextOwnerBinding,
    pub(super) draft: ContextOwnerDraftEnvelope,
    pub(super) document: ContextSourceDocument,
    pub(super) owner_state_version: u64,
    pub(super) owner_state_bytes: Vec<u8>,
    pub(super) expires_at: u64,
}

impl Owner {
    pub(super) fn draft_publication_candidate(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        entry: &Current,
        request: &ContextOwnerDraftPublicationRequest,
        now: u64,
    ) -> Result<DraftPublicationCandidate, ManagementError> {
        let binding = self.authorize_current_binding(actor, snapshot, entry, false)?;
        if !binding.grants.content_read {
            return Err(ManagementError::capability(
                "context_publication_content_unavailable",
                "current binding does not grant access to publish context content",
            ));
        }
        if request.expected_boundary != binding.boundary
            || request.expected_binding_id != binding.binding_id
            || request.expected_binding_digest != binding.binding_digest
        {
            return Err(ManagementError::conflict(
                "context_publication_binding_stale",
                "publication request does not name the exact current owner binding",
            ));
        }
        let (state, owner_state_version) =
            self.load_draft_state(entry, &snapshot.workflow_run_id)?;
        if owner_state_version == 0 || owner_state_version != request.expected_owner_state_version {
            return Err(ManagementError::conflict(
                "context_publication_owner_state_stale",
                "publication request does not name the current durable owner-state version",
            ));
        }
        let stored = state
            .drafts
            .get(&request.draft_id)
            .filter(|draft| draft.envelope.actor_subject == actor.subject)
            .cloned()
            .ok_or_else(|| {
                ManagementError::invalid("context_draft_not_found", "owner draft was not found")
            })?;
        if stored.envelope.binding != binding
            || stored.envelope.draft.version != request.expected_draft_version
            || stored.envelope.draft.base_revision_id != request.expected_base_revision_id
            || stored.envelope.draft.base_revision_id != entry.authority.state().active_revision_id
        {
            return Err(ManagementError::conflict(
                "context_publication_draft_stale",
                "publication request does not name the exact current draft and base revision",
            ));
        }
        let active = self.verify_active_source(entry, &stored)?;
        let registry = self.eligible_registry(entry, &state, Some(&stored))?;
        let draft = &stored.envelope.draft;
        let references = draft
            .selected_items
            .iter()
            .chain(draft.notes.iter().map(|note| &note.reference))
            .chain(draft.objective.iter());
        let mut items = BTreeMap::new();
        for reference in references {
            let key = item_key(reference);
            let eligible = registry.get(&key).ok_or_else(item_unavailable)?;
            if eligible.item.reference != *reference || eligible.item.expires_at <= now {
                return Err(ManagementError::conflict(
                    "context_publication_item_stale",
                    "a selected draft item is missing, changed, or expired",
                ));
            }
            items.insert(key, eligible.item.clone());
        }
        let document = ContextSourceDocument {
            draft: draft.clone(),
            items,
        };
        validate_document(&document)?;
        let retention_expiry = stored.envelope.retention_expires_at.unwrap_or(u64::MAX);
        let source_expiry = source_valid_until(&document);
        let expires_at = retention_expiry.min(source_expiry);
        if expires_at == u64::MAX || expires_at <= now {
            return Err(ManagementError::conflict(
                "context_publication_expired",
                "publication requires a finite future source and draft-retention horizon",
            ));
        }
        if active.is_some_and(|(_, _, active_expiry)| active_expiry <= now) {
            return Err(ManagementError::conflict(
                "context_publication_source_expired",
                "the draft base source is no longer current",
            ));
        }
        let owner_state_bytes = state.encode()?;
        Ok(DraftPublicationCandidate {
            binding,
            draft: stored.envelope,
            document,
            owner_state_version,
            owner_state_bytes,
            expires_at,
        })
    }
}
