// SPDX-License-Identifier: MIT

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
use sts2_harness::context_control::{ContextSourceDocument, context_source_digest};

pub(super) fn validate_document(
    document: &ContextSourceDocument,
) -> Result<String, ManagementError> {
    if document.draft.schema != sts2_harness::context_control::CONTEXT_DRAFT_SCHEMA
        || document.draft.version == 0
        || document.items.len() > sts2_harness::context_control::MAX_CONTEXT_ITEMS
        || document.draft.selected_items.len() > sts2_harness::context_control::MAX_CONTEXT_ITEMS
        || document.draft.notes.len() > sts2_harness::context_control::MAX_CONTEXT_NOTES
    {
        return Err(ManagementError::invalid(
            "context_source_invalid",
            "context source draft or item registry exceeds its schema bounds",
        ));
    }
    sts2_harness::management::validate_identifier(
        "context_source_draft",
        &document.draft.draft_id,
    )?;
    sts2_harness::management::validate_identifier(
        "context_source_revision",
        &document.draft.base_revision_id,
    )?;
    sts2_harness::management::validate_identifier(
        "context_source_author",
        &document.draft.author_ref,
    )?;
    let mut byte_total = 0_usize;
    for (key, item) in &document.items {
        if !item.reference.valid()
            || key != &format!("{}:{}", item.reference.item_id, item.reference.version)
            || item.bytes.is_empty()
            || item.bytes.len() > sts2_harness::context_control::MAX_CONTEXT_BYTES
            || sts2_harness::sha256_hex(&item.bytes) != item.reference.sha256
        {
            return Err(ManagementError::invalid(
                "context_source_item_invalid",
                "context source contains an invalid item identity or digest",
            ));
        }
        sts2_harness::management::validate_identifier("context_source_item_kind", &item.kind)?;
        byte_total = byte_total.saturating_add(item.bytes.len());
    }
    if byte_total > sts2_harness::context_control::MAX_CONTEXT_BYTES {
        return Err(ManagementError::invalid(
            "context_source_too_large",
            "context source item content exceeds the owner bound",
        ));
    }
    for reference in document
        .draft
        .selected_items
        .iter()
        .chain(document.draft.notes.iter().map(|note| &note.reference))
        .chain(document.draft.objective.iter())
    {
        let key = format!("{}:{}", reference.item_id, reference.version);
        if !reference.valid()
            || document
                .items
                .get(&key)
                .is_none_or(|item| item.reference != *reference)
        {
            return Err(ManagementError::invalid(
                "context_source_reference_invalid",
                "context source draft references an item that is not in its immutable registry",
            ));
        }
    }
    context_source_digest(document)
        .map_err(|error| ManagementError::invalid("context_source_invalid", error.to_string()))
}

pub(super) fn source_valid_until(document: &ContextSourceDocument) -> u64 {
    document
        .draft
        .selected_items
        .iter()
        .map(|reference| format!("{}:{}", reference.item_id, reference.version))
        .chain(
            document
                .draft
                .notes
                .iter()
                .map(|note| format!("{}:{}", note.reference.item_id, note.reference.version)),
        )
        .chain(
            document
                .draft
                .objective
                .iter()
                .map(|reference| format!("{}:{}", reference.item_id, reference.version)),
        )
        .filter_map(|key| document.items.get(&key).map(|item| item.expires_at))
        .min()
        .unwrap_or(u64::MAX)
}

pub(super) fn unix_time() -> Result<u64, ManagementError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| {
            ManagementError::unavailable(
                "context_owner_clock_unavailable",
                "system clock is before the Unix epoch",
            )
        })
}
