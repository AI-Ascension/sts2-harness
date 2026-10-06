// SPDX-License-Identifier: MIT

struct PatchOperationContext<'a> {
    owner: &'a Owner,
    state: &'a mut OwnerDraftState,
    stored: &'a mut StoredDraft,
    actor: &'a AuthContext,
    draft_id: &'a str,
    registry: &'a BTreeMap<String, EligibleItem>,
    active: Option<&'a (SourceIdentity, ContextSourceDocument, u64)>,
    finite_horizon: Option<u64>,
    now: u64,
}

struct PatchFinalization<'a> {
    actor: &'a AuthContext,
    snapshot: &'a sts2_harness::management::RunSnapshot,
    binding: &'a ContextOwnerBinding,
    request: &'a ContextOwnerDraftPatchRequest,
    payload_digest: String,
    now: u64,
}

impl Owner {
    pub(super) fn patch_draft_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &ContextOwnerDraftPatchRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        validate_patch_request(request)?;
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
        authorize_patch_grants(actor, &binding, &request.operations)?;
        if request.expected_boundary != binding.boundary {
            return Err(stale_boundary());
        }
        let mut stored = current_patch_draft(&state, request, actor, &binding, entry)?;
        let now = unix_time()?;
        let active = self.verify_active_source(entry, &stored)?;
        let finite_horizon = check_patch_horizon(&stored, &active, &request.operations, now)?;
        let registry = self.eligible_registry(entry, &state, Some(&stored))?;
        let next_version = stored
            .envelope
            .draft
            .version
            .checked_add(1)
            .ok_or_else(owner_capacity)?;
        {
            let mut context = PatchOperationContext {
                owner: self,
                state: &mut state,
                stored: &mut stored,
                actor,
                draft_id: &request.draft_id,
                registry: &registry,
                active: active.as_ref(),
                finite_horizon,
                now,
            };
            apply_patch_operations(&mut context, &request.operations)?;
        }
        prune_authored_items(&mut state, &mut stored, &request.draft_id);
        validate_patched_draft(self, actor, entry, &state, &stored, &binding, now)?;
        stored.envelope.draft.version = next_version;
        stored.envelope.updated_at = now;
        stored.envelope.binding = binding.clone();
        let receipt = self.finish_patch(
            entry,
            &mut state,
            record_version,
            PatchFinalization {
                actor,
                snapshot,
                binding: &binding,
                request,
                payload_digest,
                now,
            },
            stored,
        )?;
        Ok(receipt)
    }

    fn finish_patch(
        &self,
        entry: &mut Current,
        state: &mut OwnerDraftState,
        record_version: u64,
        finalization: PatchFinalization<'_>,
        stored: StoredDraft,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        let PatchFinalization {
            actor,
            snapshot,
            binding,
            request,
            payload_digest,
            now,
        } = finalization;
        let revision = self.new_revision(
            state,
            actor,
            binding,
            &stored.envelope.draft,
            now,
            stored.envelope.retention_expires_at,
        )?;
        state
            .revisions
            .insert(revision.revision_id.clone(), revision);
        state
            .drafts
            .insert(request.draft_id.clone(), stored.clone());
        let receipt = self.new_receipt(MutationReceiptInput {
            actor,
            workflow_run_id: &snapshot.workflow_run_id,
            binding,
            operation: "patch_draft",
            request_id: request.request_id.clone(),
            payload_digest,
            result: ContextOwnerMutationResult::Draft(stored.envelope),
            now,
        });
        self.record_receipt(state, receipt.clone())?;
        self.persist_draft_state(entry, state, record_version)?;
        Ok(receipt)
    }
}

fn current_patch_draft(
    state: &OwnerDraftState,
    request: &ContextOwnerDraftPatchRequest,
    actor: &AuthContext,
    binding: &ContextOwnerBinding,
    entry: &Current,
) -> Result<StoredDraft, ManagementError> {
    let stored = state
        .drafts
        .get(&request.draft_id)
        .cloned()
        .filter(|draft| draft.envelope.actor_subject == actor.subject)
        .ok_or_else(|| {
            ManagementError::invalid("context_draft_not_found", "owner draft was not found")
        })?;
    if stored.envelope.binding != *binding
        || stored.envelope.draft.version != request.expected_version
        || stored.envelope.draft.base_revision_id != entry.authority.state().active_revision_id
    {
        return Err(ManagementError::conflict(
            "context_draft_stale",
            "draft revision or full owner binding changed",
        ));
    }
    Ok(stored)
}

fn authorize_patch_grants(
    actor: &AuthContext,
    binding: &ContextOwnerBinding,
    operations: &[ContextOwnerDraftOperation],
) -> Result<(), ManagementError> {
    let changes_objective = operations.iter().any(|operation| {
        matches!(
            operation,
            ContextOwnerDraftOperation::SetObjective { .. }
                | ContextOwnerDraftOperation::RemoveObjective
        )
    });
    let changes_other = operations.iter().any(|operation| {
        !matches!(
            operation,
            ContextOwnerDraftOperation::SetObjective { .. }
                | ContextOwnerDraftOperation::RemoveObjective
        )
    });
    if changes_objective && !actor.can("workflow:context:objective:edit") {
        return Err(ManagementError::forbidden(
            "context_owner_objective_forbidden",
            "objective editing requires its independent owner grant",
        ));
    }
    if !binding.grants.edit {
        return Err(ManagementError::capability(
            "context_owner_edit_unavailable",
            "current owner binding does not grant context draft changes",
        ));
    }
    if changes_other && !actor.can("workflow:context:edit") {
        return Err(ManagementError::forbidden(
            "context_owner_edit_forbidden",
            "draft and note changes require the independent context edit grant",
        ));
    }
    Ok(())
}

fn check_patch_horizon(
    stored: &StoredDraft,
    active: &Option<(SourceIdentity, ContextSourceDocument, u64)>,
    operations: &[ContextOwnerDraftOperation],
    now: u64,
) -> Result<Option<u64>, ManagementError> {
    let finite_horizon = active
        .as_ref()
        .and_then(|(_, _, valid_until)| (*valid_until != u64::MAX).then_some(*valid_until));
    let adds_text = operations.iter().any(|operation| {
        matches!(
            operation,
            ContextOwnerDraftOperation::PutNote { .. }
                | ContextOwnerDraftOperation::SetObjective { .. }
        )
    });
    if adds_text {
        let Some(valid_until) = finite_horizon else {
            return Err(ManagementError::capability(
                "context_owner_authored_retention_unavailable",
                "new note or objective text needs a finite horizon from the exact active owner source",
            ));
        };
        if valid_until <= now || stored.envelope.retention_expires_at != Some(valid_until) {
            return Err(ManagementError::conflict(
                "context_owner_authored_retention_expired",
                "the draft's exact active source no longer grants a live finite text-retention horizon",
            ));
        }
    }
    Ok(finite_horizon)
}

fn validate_authored_retention(
    stored: &StoredDraft,
    state: &OwnerDraftState,
    finite_horizon: Option<u64>,
    now: u64,
) -> Result<(), ManagementError> {
    if !draft_retains_authored_bytes(state, stored) {
        return Ok(());
    }
    let Some(valid_until) = finite_horizon else {
        return Err(ManagementError::conflict(
            "context_owner_authored_retention_unavailable",
            "draft-authored bytes lost their exact finite active-source horizon",
        ));
    };
    if valid_until <= now || stored.envelope.retention_expires_at != Some(valid_until) {
        return Err(ManagementError::conflict(
            "context_owner_authored_retention_expired",
            "draft-authored bytes are outside their original active-source retention horizon",
        ));
    }
    Ok(())
}

fn draft_retains_authored_bytes(state: &OwnerDraftState, stored: &StoredDraft) -> bool {
    stored.envelope.draft.notes.iter().any(|note| {
        state
            .authored_items
            .contains_key(&item_key(&note.reference))
    }) || stored
        .envelope
        .draft
        .objective
        .as_ref()
        .is_some_and(|reference| state.authored_items.contains_key(&item_key(reference)))
}
