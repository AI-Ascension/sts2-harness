// SPDX-License-Identifier: MIT

fn validate_patched_draft(
    owner: &Owner,
    actor: &AuthContext,
    entry: &Current,
    state: &OwnerDraftState,
    stored: &StoredDraft,
    binding: &ContextOwnerBinding,
    now: u64,
) -> Result<(), ManagementError> {
    let descriptor = ContextOwnerEffectiveLimitsView::compose(&owner.catalog(actor)?, binding)?;
    validate_draft_shape(stored, &descriptor)?;
    validate_draft_content(owner, entry, state, stored, actor, &descriptor)?;
    let active = owner.verify_active_source(entry, stored)?;
    let finite_horizon = active
        .as_ref()
        .and_then(|(_, _, until)| (*until != u64::MAX).then_some(*until));
    validate_authored_retention(stored, state, finite_horizon, now)
}

fn validate_draft_shape(
    stored: &StoredDraft,
    descriptor: &ContextOwnerEffectiveLimitsView,
) -> Result<(), ManagementError> {
    let draft = &stored.envelope.draft;
    if draft.selected_items.len() > descriptor.effective_limits.max_items as usize
        || draft.selected_items.len() > MAX_CONTEXT_ITEMS
        || draft.notes.len() > descriptor.effective_limits.max_notes as usize
        || draft.notes.len() > MAX_CONTEXT_NOTES
        || draft.pinned_item_ids.len() > draft.selected_items.len()
        || draft.pinned_item_ids.iter().any(|item_id| {
            !draft
                .selected_items
                .iter()
                .any(|reference| &reference.item_id == item_id)
        })
    {
        return Err(ManagementError::invalid(
            "context_owner_draft_bounds",
            "draft selections, notes, or pins exceed the admitted owner limits",
        ));
    }
    Ok(())
}

fn validate_draft_content(
    owner: &Owner,
    entry: &Current,
    state: &OwnerDraftState,
    stored: &StoredDraft,
    actor: &AuthContext,
    descriptor: &ContextOwnerEffectiveLimitsView,
) -> Result<(), ManagementError> {
    let registry = owner
        .eligible_registry(entry, state, Some(stored))?
        .into_iter()
        .map(|(key, item)| (key, item.item))
        .collect::<BTreeMap<_, _>>();
    let selected_bytes = selected_content_bytes(&stored.envelope.draft, &registry)?;
    let note_bytes = note_content_bytes(&stored.envelope.draft, &registry, actor)?;
    let objective_bytes = objective_content_bytes(&stored.envelope.draft, &registry, descriptor)?;
    let total = selected_bytes
        .checked_add(note_bytes)
        .and_then(|bytes| bytes.checked_add(objective_bytes))
        .ok_or_else(owner_capacity)?;
    if total > descriptor.effective_limits.max_context_bytes as usize
        || total > MAX_CONTEXT_BYTES
    {
        return Err(ManagementError::invalid(
            "context_owner_draft_content_too_large",
            "draft content exceeds the selected owner or harness byte bound",
        ));
    }
    Ok(())
}

fn selected_content_bytes(
    draft: &ContextDraft,
    registry: &BTreeMap<String, ContextItem>,
) -> Result<usize, ManagementError> {
    draft
        .selected_items
        .iter()
        .try_fold(0_usize, |total, reference| {
            let item = exact_item(registry, reference)?;
            total.checked_add(item.bytes.len()).ok_or_else(owner_capacity)
        })
}

fn note_content_bytes(
    draft: &ContextDraft,
    registry: &BTreeMap<String, ContextItem>,
    actor: &AuthContext,
) -> Result<usize, ManagementError> {
    draft.notes.iter().try_fold(0_usize, |total, note| {
        let item = exact_item(registry, &note.reference)?;
        if item.bytes.len() > MAX_NOTE_BYTES || note.attributed_to != actor.subject {
            return Err(ManagementError::invalid(
                "context_owner_note_invalid",
                "note bytes or authenticated attribution are outside the owner bound",
            ));
        }
        total.checked_add(item.bytes.len()).ok_or_else(owner_capacity)
    })
}

fn objective_content_bytes(
    draft: &ContextDraft,
    registry: &BTreeMap<String, ContextItem>,
    descriptor: &ContextOwnerEffectiveLimitsView,
) -> Result<usize, ManagementError> {
    let Some(reference) = &draft.objective else {
        return Ok(0);
    };
    let item = exact_item(registry, reference)?;
    if item.bytes.len() > descriptor.effective_limits.max_objective_bytes as usize
        || item.bytes.len() > MAX_OBJECTIVE_BYTES
    {
        return Err(ManagementError::invalid(
            "context_owner_objective_size",
            "objective bytes exceed the admitted owner bound",
        ));
    }
    Ok(item.bytes.len())
}

fn exact_item<'a>(
    registry: &'a BTreeMap<String, ContextItem>,
    reference: &ContextItemRef,
) -> Result<&'a ContextItem, ManagementError> {
    registry
        .get(&item_key(reference))
        .filter(|item| item.reference == *reference)
        .ok_or_else(item_unavailable)
}
