// SPDX-License-Identifier: MIT

//! Draft admission and context selection for the managed renderer.
//!
//! Everything between an authored draft and the managed-context document it becomes: the
//! structural limits, the per-boundary budgets, and the resolution of each selected reference and
//! attributed note against the catalog. It is separated from the request assembly that follows so
//! the admission rules can be read, and changed, as one unit.

use super::super::types::{
    ContextDraft, ContextItem, MAX_CONTEXT_ITEMS, MAX_CONTEXT_NOTES, MAX_NOTE_BYTES,
    MAX_OBJECTIVE_BYTES,
};
use super::{
    ContextRenderError, ContextRenderLimits, ManagedRenderInput, digest, reference_key,
    valid_attributed_to, validate_request,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// A draft that passed every admission rule, and the managed-context values it yields.
pub(super) struct AdmittedDraft {
    pub(super) selected: Vec<Value>,
    pub(super) notes: Vec<Value>,
    pub(super) objective_text: String,
}

pub(super) fn admit(
    request: &ManagedRenderInput,
    draft: &ContextDraft,
    registry: &BTreeMap<String, ContextItem>,
    now: u64,
    limits: &ContextRenderLimits,
) -> Result<AdmittedDraft, ContextRenderError> {
    validate_request(&request)?;
    if draft.selected_items.len() > MAX_CONTEXT_ITEMS || draft.notes.len() > MAX_CONTEXT_NOTES {
        return Err(ContextRenderError::TooLarge);
    }
    if draft.selected_items.len() > limits.max_items {
        return Err(ContextRenderError::ExceedsSelectedLimit("max_items"));
    }
    if draft.notes.len() > limits.max_notes {
        return Err(ContextRenderError::ExceedsSelectedLimit("max_notes"));
    }
    if draft
        .selected_items
        .iter()
        .any(|reference| !reference.valid())
        || draft
            .notes
            .iter()
            .any(|note| !note.reference.valid() || !valid_attributed_to(&note.attributed_to))
        || draft
            .objective
            .as_ref()
            .is_some_and(|reference| !reference.valid())
    {
        return Err(ContextRenderError::InvalidInput(
            "context reference is invalid",
        ));
    }
    if draft.pinned_item_ids.len() > draft.selected_items.len()
        || draft.pinned_item_ids.iter().any(|item_id| {
            !draft
                .selected_items
                .iter()
                .any(|reference| reference.item_id == *item_id)
        })
    {
        return Err(ContextRenderError::InvalidInput(
            "pinned item is not selected",
        ));
    }
    let mut selected = Vec::with_capacity(draft.selected_items.len());
    for reference in &draft.selected_items {
        let item = registry
            .get(&reference_key(reference))
            .ok_or(ContextRenderError::UnknownItem)?;
        if item.reference != *reference || digest(&item.bytes) != reference.sha256 {
            return Err(ContextRenderError::UnknownItem);
        }
        if item.protected {
            return Err(ContextRenderError::ProtectedItem);
        }
        if !item.editable(now) {
            return Err(ContextRenderError::ExpiredItem);
        }
        let content =
            std::str::from_utf8(&item.bytes).map_err(|_| ContextRenderError::InvalidUtf8)?;
        selected.push(json!({
            "item_id": item.reference.item_id,
            "version": item.reference.version,
            "sha256": item.reference.sha256,
            "kind": item.kind,
            "content": content,
        }));
    }
    let notes = draft
        .notes
        .iter()
        .map(|note| {
            let item = registry
                .get(&reference_key(&note.reference))
                .ok_or(ContextRenderError::UnknownItem)?;
            if item.reference != note.reference {
                return Err(ContextRenderError::ExpiredItem);
            }
            if digest(&item.bytes) != note.reference.sha256 {
                return Err(ContextRenderError::UnknownItem);
            }
            if !item.editable(now) {
                return Err(ContextRenderError::ExpiredItem);
            }
            if item.bytes.len() > MAX_NOTE_BYTES {
                return Err(ContextRenderError::TooLarge);
            }
            let content =
                std::str::from_utf8(&item.bytes).map_err(|_| ContextRenderError::InvalidUtf8)?;
            Ok(json!({
                "item_id": item.reference.item_id,
                "version": item.reference.version,
                "sha256": item.reference.sha256,
                "attributed_to": note.attributed_to,
                "content": content,
            }))
        })
        .collect::<Result<Vec<_>, ContextRenderError>>()?;
    let objective = draft
        .objective
        .as_ref()
        .map(|reference| {
            let item = registry
                .get(&reference_key(reference))
                .ok_or(ContextRenderError::UnknownItem)?;
            if item.reference != *reference {
                return Err(ContextRenderError::ExpiredItem);
            }
            if digest(&item.bytes) != reference.sha256 {
                return Err(ContextRenderError::UnknownItem);
            }
            if !item.editable(now) {
                return Err(ContextRenderError::ExpiredItem);
            }
            if item.bytes.len() > MAX_OBJECTIVE_BYTES {
                return Err(ContextRenderError::TooLarge);
            }
            std::str::from_utf8(&item.bytes)
                .map(str::to_owned)
                .map_err(|_| ContextRenderError::InvalidUtf8)
        })
        .transpose()?;
    let objective_text = objective.unwrap_or_else(|| request.objective.clone());
    if objective_text.len() > limits.max_objective_bytes {
        return Err(ContextRenderError::ExceedsSelectedLimit(
            "max_objective_bytes",
        ));
    }
    Ok(AdmittedDraft {
        selected,
        notes,
        objective_text,
    })
}
