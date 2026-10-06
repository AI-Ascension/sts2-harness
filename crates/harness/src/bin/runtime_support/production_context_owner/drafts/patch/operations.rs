// SPDX-License-Identifier: MIT

fn apply_patch_operations(
    context: &mut PatchOperationContext<'_>,
    operations: &[ContextOwnerDraftOperation],
) -> Result<(), ManagementError> {
    for operation in operations {
        match operation {
            ContextOwnerDraftOperation::IncludeItem { .. }
            | ContextOwnerDraftOperation::ExcludeItem { .. }
            | ContextOwnerDraftOperation::PinItem { .. }
            | ContextOwnerDraftOperation::UnpinItem { .. } => {
                apply_selection_operation(
                    &mut *context.stored,
                    operation,
                    context.registry,
                    context.now,
                )?;
            }
            ContextOwnerDraftOperation::PutNote { .. }
            | ContextOwnerDraftOperation::RemoveNote { .. } => {
                apply_note_operation(context, operation)?;
            }
            ContextOwnerDraftOperation::SetObjective { .. }
            | ContextOwnerDraftOperation::RemoveObjective => {
                apply_objective_operation(context, operation)?;
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
        ContextOwnerDraftOperation::PinItem { item_id } => pin_item(stored, item_id, registry),
        ContextOwnerDraftOperation::UnpinItem { item_id } => {
            validate_identifier("context_item_id", item_id)?;
            stored
                .envelope
                .draft
                .pinned_item_ids
                .retain(|id| id != item_id);
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
    let item = registry
        .get(&item_key(reference))
        .ok_or_else(item_unavailable)?;
    if item.item.reference != *reference || item.item.protected {
        return Err(ManagementError::capability(
            "context_owner_item_protected",
            "a protected owner prerequisite cannot be excluded",
        ));
    }
    let before = stored.envelope.draft.selected_items.len();
    stored
        .envelope
        .draft
        .selected_items
        .retain(|candidate| candidate != reference);
    if before == stored.envelope.draft.selected_items.len() {
        return Err(item_unavailable());
    }
    stored
        .envelope
        .draft
        .pinned_item_ids
        .retain(|id| id != &reference.item_id);
    stored
        .envelope
        .draft
        .notes
        .retain(|note| note.reference != *reference);
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
        .find(|reference| reference.item_id == item_id)
    else {
        return Err(ManagementError::conflict(
            "context_owner_pin_not_selected",
            "a pin must name an item in the effective selected set",
        ));
    };
    if registry
        .get(&item_key(reference))
        .is_none_or(|item| item.item.protected)
    {
        return Err(ManagementError::capability(
            "context_owner_pin_protected",
            "protected prerequisites cannot be pinned as model-visible items",
        ));
    }
    if !stored
        .envelope
        .draft
        .pinned_item_ids
        .iter()
        .any(|pinned_id| pinned_id.as_str() == item_id)
    {
        stored
            .envelope
            .draft
            .pinned_item_ids
            .push(item_id.to_owned());
    }
    Ok(())
}
