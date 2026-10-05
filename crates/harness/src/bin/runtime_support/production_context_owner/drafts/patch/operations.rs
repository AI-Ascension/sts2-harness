// SPDX-License-Identifier: MIT

fn apply_patch_operations(
    owner: &Owner,
    state: &mut OwnerDraftState,
    stored: &mut StoredDraft,
    actor: &AuthContext,
    request: &ContextOwnerDraftPatchRequest,
    operations: &[ContextOwnerDraftOperation],
    registry: &BTreeMap<String, EligibleItem>,
    active: Option<&(SourceIdentity, ContextSourceDocument, u64)>,
    finite_horizon: Option<u64>,
    now: u64,
) -> Result<(), ManagementError> {
    for operation in operations {
        match operation {
            ContextOwnerDraftOperation::IncludeItem { .. }
            | ContextOwnerDraftOperation::ExcludeItem { .. }
            | ContextOwnerDraftOperation::PinItem { .. }
            | ContextOwnerDraftOperation::UnpinItem { .. } => {
                apply_selection_operation(stored, operation, registry, now)?;
            }
            ContextOwnerDraftOperation::PutNote { .. }
            | ContextOwnerDraftOperation::RemoveNote { .. } => {
                apply_note_operation(
                    owner,
                    state,
                    stored,
                    actor,
                    &request.draft_id,
                    operation,
                    active,
                    finite_horizon,
                )?;
            }
            ContextOwnerDraftOperation::SetObjective { .. }
            | ContextOwnerDraftOperation::RemoveObjective => {
                apply_objective_operation(
                    owner,
                    state,
                    stored,
                    actor,
                    &request.draft_id,
                    operation,
                    active,
                    finite_horizon,
                )?;
            }
        }
    }
    Ok(())
}

fn apply_selection_operation(
    stored: &mut StoredDraft,
    operation: &ContextOwnerDraftOperation,
    registry: &BTreeMap<String, EligibleItem>,
    now: u64,
) -> Result<(), ManagementError> {
    match operation {
        ContextOwnerDraftOperation::IncludeItem { reference } => {
            include_item(stored, reference, registry, now)
        }
        ContextOwnerDraftOperation::ExcludeItem { reference } => {
            exclude_item(stored, reference, registry)
        }
        ContextOwnerDraftOperation::PinItem { item_id } => {
            pin_item(stored, item_id, registry)
        }
        ContextOwnerDraftOperation::UnpinItem { item_id } => {
            validate_identifier("context_item_id", item_id)?;
            stored.envelope.draft.pinned_item_ids.retain(|id| id != item_id);
            Ok(())
        }
        _ => Err(ManagementError::invalid(
            "context_owner_operation_invalid",
            "operation does not belong to the selection operation group",
        )),
    }
}

fn include_item(
    stored: &mut StoredDraft,
    reference: &ContextItemRef,
    registry: &BTreeMap<String, EligibleItem>,
    now: u64,
) -> Result<(), ManagementError> {
    let key = item_key(reference);
    if stored.authored_item_keys.contains(&key) {
        return Err(item_unavailable());
    }
    let item = registry.get(&key).ok_or_else(item_unavailable)?;
    if item.item.reference != *reference || item.item.protected || item.item.expires_at <= now {
        return Err(item_unavailable());
    }
    if !stored.envelope.draft.selected_items.contains(reference) {
        stored.envelope.draft.selected_items.push(reference.clone());
    }
    Ok(())
}

fn exclude_item(
    stored: &mut StoredDraft,
    reference: &ContextItemRef,
    registry: &BTreeMap<String, EligibleItem>,
) -> Result<(), ManagementError> {
    let item = registry.get(&item_key(reference)).ok_or_else(item_unavailable)?;
    if item.item.reference != *reference || item.item.protected {
        return Err(ManagementError::capability(
            "context_owner_item_protected",
            "a protected owner prerequisite cannot be excluded",
        ));
    }
    let before = stored.envelope.draft.selected_items.len();
    stored.envelope.draft.selected_items.retain(|candidate| candidate != reference);
    if before == stored.envelope.draft.selected_items.len() {
        return Err(item_unavailable());
    }
    stored.envelope.draft.pinned_item_ids.retain(|id| id != &reference.item_id);
    stored.envelope.draft.notes.retain(|note| note.reference != *reference);
    Ok(())
}

fn pin_item(
    stored: &mut StoredDraft,
    item_id: &str,
    registry: &BTreeMap<String, EligibleItem>,
) -> Result<(), ManagementError> {
    validate_identifier("context_item_id", item_id)?;
    let Some(reference) = stored
        .envelope
        .draft
        .selected_items
        .iter()
        .find(|reference| &reference.item_id == item_id)
    else {
        return Err(ManagementError::conflict(
            "context_owner_pin_not_selected",
            "a pin must name an item in the effective selected set",
        ));
    };
    if registry.get(&item_key(reference)).is_none_or(|item| item.item.protected) {
        return Err(ManagementError::capability(
            "context_owner_pin_protected",
            "protected prerequisites cannot be pinned as model-visible items",
        ));
    }
    if !stored.envelope.draft.pinned_item_ids.contains(item_id) {
        stored.envelope.draft.pinned_item_ids.push(item_id.to_owned());
    }
    Ok(())
}

fn apply_note_operation(
    owner: &Owner,
    state: &mut OwnerDraftState,
    stored: &mut StoredDraft,
    actor: &AuthContext,
    draft_id: &str,
    operation: &ContextOwnerDraftOperation,
    active: Option<&(SourceIdentity, ContextSourceDocument, u64)>,
    finite_horizon: Option<u64>,
) -> Result<(), ManagementError> {
    match operation {
        ContextOwnerDraftOperation::PutNote { note_id, text } => owner.put_note(
            state,
            stored,
            actor,
            draft_id,
            note_id,
            text,
            active,
            finite_horizon,
        ),
        ContextOwnerDraftOperation::RemoveNote { note_id } => {
            validate_identifier("context_note_id", note_id)?;
            let item_id = format!("draft-note.{draft_id}.{note_id}");
            stored
                .envelope
                .draft
                .notes
                .retain(|note| note.reference.item_id != item_id);
            Ok(())
        }
        _ => Err(ManagementError::invalid(
            "context_owner_operation_invalid",
            "operation does not belong to the note operation group",
        )),
    }
}

impl Owner {
    fn put_note(
        &self,
        state: &mut OwnerDraftState,
        stored: &mut StoredDraft,
        actor: &AuthContext,
        draft_id: &str,
        note_id: &str,
        text: &str,
        active: Option<&(SourceIdentity, ContextSourceDocument, u64)>,
        finite_horizon: Option<u64>,
    ) -> Result<(), ManagementError> {
        validate_identifier("context_note_id", note_id)?;
        if text.is_empty() || text.len() > MAX_NOTE_BYTES {
            return Err(ManagementError::invalid(
                "context_owner_note_size",
                "note text must be nonempty and within the owner byte bound",
            ));
        }
        let source = active
            .map(|(source, _, _)| source.clone())
            .ok_or_else(authored_source_unavailable)?;
        let expires_at = finite_horizon.ok_or_else(owner_capacity)?;
        let reference = self.create_authored_item(
            state,
            stored,
            actor,
            &format!("draft-note.{draft_id}.{note_id}"),
            "note",
            text.as_bytes(),
            expires_at,
            source,
        )?;
        stored
            .envelope
            .draft
            .notes
            .retain(|note| note.reference.item_id != reference.item_id);
        stored.envelope.draft.notes.push(ContextNote {
            reference,
            attributed_to: actor.subject.clone(),
        });
        Ok(())
    }
}

fn authored_source_unavailable() -> ManagementError {
    ManagementError::capability(
        "context_owner_authored_retention_unavailable",
        "new authored text needs an exact active advertised source",
    )
}

fn apply_objective_operation(
    owner: &Owner,
    state: &mut OwnerDraftState,
    stored: &mut StoredDraft,
    actor: &AuthContext,
    draft_id: &str,
    operation: &ContextOwnerDraftOperation,
    active: Option<&(SourceIdentity, ContextSourceDocument, u64)>,
    finite_horizon: Option<u64>,
) -> Result<(), ManagementError> {
    match operation {
        ContextOwnerDraftOperation::SetObjective { text } => owner.set_objective(
            state,
            stored,
            actor,
            draft_id,
            text,
            active,
            finite_horizon,
        ),
        ContextOwnerDraftOperation::RemoveObjective => {
            stored.envelope.draft.objective = None;
            Ok(())
        }
        _ => Err(ManagementError::invalid(
            "context_owner_operation_invalid",
            "operation does not belong to the objective operation group",
        )),
    }
}

impl Owner {
    fn set_objective(
        &self,
        state: &mut OwnerDraftState,
        stored: &mut StoredDraft,
        actor: &AuthContext,
        draft_id: &str,
        text: &str,
        active: Option<&(SourceIdentity, ContextSourceDocument, u64)>,
        finite_horizon: Option<u64>,
    ) -> Result<(), ManagementError> {
        if text.is_empty() || text.len() > MAX_OBJECTIVE_BYTES {
            return Err(ManagementError::invalid(
                "context_owner_objective_size",
                "objective text must be nonempty and within the owner byte bound",
            ));
        }
        let source = active
            .map(|(source, _, _)| source.clone())
            .ok_or_else(authored_source_unavailable)?;
        let expires_at = finite_horizon.ok_or_else(owner_capacity)?;
        stored.envelope.draft.objective = Some(self.create_authored_item(
            state,
            stored,
            actor,
            &format!("draft-objective.{draft_id}"),
            "objective",
            text.as_bytes(),
            expires_at,
            source,
        )?);
        Ok(())
    }
}

fn draft_item_keys(draft: &StoredDraft) -> std::collections::BTreeSet<String> {
    let mut keys = draft
        .envelope
        .draft
        .notes
        .iter()
        .map(|note| item_key(&note.reference))
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(objective) = &draft.envelope.draft.objective {
        keys.insert(item_key(objective));
    }
    keys
}

fn draft_authored_item_keys(
    draft: &StoredDraft,
    authored_keys: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    draft_item_keys(draft)
        .intersection(authored_keys)
        .cloned()
        .collect()
}

fn prune_authored_items(
    state: &mut OwnerDraftState,
    current: &mut StoredDraft,
    current_id: &str,
) {
    let authored_keys = state.authored_items.keys().cloned().collect();
    current.authored_item_keys = draft_authored_item_keys(current, &authored_keys)
        .into_iter()
        .collect();
    let mut referenced = draft_authored_item_keys(current, &authored_keys);
    for (draft_id, stored) in &mut state.drafts {
        if draft_id == current_id {
            continue;
        }
        let keys = draft_authored_item_keys(stored, &authored_keys);
        stored.authored_item_keys = keys.iter().cloned().collect();
        referenced.extend(keys);
    }
    state.authored_items.retain(|key, _| referenced.contains(key));
}
