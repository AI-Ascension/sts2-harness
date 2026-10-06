// SPDX-License-Identifier: MIT

fn apply_note_operation(
    context: &mut PatchOperationContext<'_>,
    operation: &ContextOwnerDraftOperation,
) -> Result<(), ManagementError> {
    match operation {
        ContextOwnerDraftOperation::PutNote { note_id, text } => put_note(context, note_id, text),
        ContextOwnerDraftOperation::RemoveNote { note_id } => {
            validate_identifier("context_note_id", note_id)?;
            let item_id = format!("draft-note.{}.{note_id}", context.draft_id);
            context
                .stored
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

fn put_note(
    context: &mut PatchOperationContext<'_>,
    note_id: &str,
    text: &str,
) -> Result<(), ManagementError> {
    validate_identifier("context_note_id", note_id)?;
    if text.is_empty() || text.len() > MAX_NOTE_BYTES {
        return Err(ManagementError::invalid(
            "context_owner_note_size",
            "note text must be nonempty and within the owner byte bound",
        ));
    }
    let source = context
        .active
        .map(|(source, _, _)| source.clone())
        .ok_or_else(authored_source_unavailable)?;
    let expires_at = context.finite_horizon.ok_or_else(owner_capacity)?;
    let item_id = format!("draft-note.{}.{}", context.draft_id, note_id);
    let reference = context.owner.create_authored_item(
        &mut *context.state,
        &mut *context.stored,
        AuthoredItemInput {
            actor: context.actor,
            item_id: &item_id,
            kind: "note",
            bytes: text.as_bytes(),
            expires_at,
            source,
        },
    )?;
    context
        .stored
        .envelope
        .draft
        .notes
        .retain(|note| note.reference.item_id != reference.item_id);
    context.stored.envelope.draft.notes.push(ContextNote {
        reference,
        attributed_to: context.actor.subject.clone(),
    });
    Ok(())
}

fn authored_source_unavailable() -> ManagementError {
    ManagementError::capability(
        "context_owner_authored_retention_unavailable",
        "new authored text needs an exact active advertised source",
    )
}

fn apply_objective_operation(
    context: &mut PatchOperationContext<'_>,
    operation: &ContextOwnerDraftOperation,
) -> Result<(), ManagementError> {
    match operation {
        ContextOwnerDraftOperation::SetObjective { text } => set_objective(context, text),
        ContextOwnerDraftOperation::RemoveObjective => {
            context.stored.envelope.draft.objective = None;
            Ok(())
        }
        _ => Err(ManagementError::invalid(
            "context_owner_operation_invalid",
            "operation does not belong to the objective operation group",
        )),
    }
}

fn set_objective(
    context: &mut PatchOperationContext<'_>,
    text: &str,
) -> Result<(), ManagementError> {
    if text.is_empty() || text.len() > MAX_OBJECTIVE_BYTES {
        return Err(ManagementError::invalid(
            "context_owner_objective_size",
            "objective text must be nonempty and within the owner byte bound",
        ));
    }
    let source = context
        .active
        .map(|(source, _, _)| source.clone())
        .ok_or_else(authored_source_unavailable)?;
    let expires_at = context.finite_horizon.ok_or_else(owner_capacity)?;
    let item_id = format!("draft-objective.{}", context.draft_id);
    let objective = context.owner.create_authored_item(
        &mut *context.state,
        &mut *context.stored,
        AuthoredItemInput {
            actor: context.actor,
            item_id: &item_id,
            kind: "objective",
            bytes: text.as_bytes(),
            expires_at,
            source,
        },
    )?;
    context.stored.envelope.draft.objective = Some(objective);
    Ok(())
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

fn prune_authored_items(state: &mut OwnerDraftState, current: &mut StoredDraft, current_id: &str) {
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
    state
        .authored_items
        .retain(|key, _| referenced.contains(key));
}
