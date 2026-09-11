// SPDX-License-Identifier: MIT

use super::types::{
    MAX_CATALOG_BYTES, MAX_OPERATION_ACTION_BYTES, MAX_ORIGINAL_CONTEXT_BYTES, OperationIntent,
    OperationState, StoredOperation, valid_digest, valid_reference,
};

/// Every operation query uses this projection. SQLite computes the source BLOB length and only
/// returns the first `MAX_OPERATION_ACTION_BYTES + 1` bytes, so hostile rows cannot force an
/// unbounded Rust allocation before the row is rejected.
pub(super) fn operation_select(suffix: &str) -> String {
    format!(
        "SELECT operation_id, run_id, episode_id, attempt_id, trajectory_id, state_id,
         generation, action_id,
         CASE WHEN action_payload IS NULL THEN NULL
              WHEN typeof(action_payload) = 'blob'
              THEN substr(action_payload, 1, {limit_plus_one})
              ELSE NULL END AS action_payload,
         CASE WHEN action_payload IS NULL THEN 0
              WHEN typeof(action_payload) <> 'blob' THEN 1
              WHEN length(action_payload) > {limit} THEN 2
              ELSE 0 END AS action_payload_invalid,
         action_kind, payload_digest, input_digest, catalog_digest,
         CASE WHEN catalog_raw IS NULL THEN NULL
              WHEN typeof(catalog_raw) = 'blob'
              THEN substr(catalog_raw, 1, {catalog_limit_plus_one}) ELSE NULL END AS catalog_raw,
         CASE WHEN catalog_raw IS NULL THEN 0
              WHEN typeof(catalog_raw) <> 'blob' THEN 1
              WHEN length(catalog_raw) > {catalog_limit} THEN 2
              ELSE 0 END AS catalog_raw_invalid,
         CASE WHEN original_context IS NULL THEN NULL
              WHEN typeof(original_context) = 'blob'
              THEN substr(original_context, 1, {context_limit_plus_one}) ELSE NULL END AS original_context,
         CASE WHEN original_context IS NULL THEN 0
              WHEN typeof(original_context) <> 'blob' THEN 1
              WHEN length(original_context) > {context_limit} THEN 2
              ELSE 0 END AS original_context_invalid,
         state,
         result_ref, result_digest
         FROM operations {suffix}",
        limit = MAX_OPERATION_ACTION_BYTES,
        limit_plus_one = MAX_OPERATION_ACTION_BYTES + 1,
        catalog_limit = MAX_CATALOG_BYTES,
        catalog_limit_plus_one = MAX_CATALOG_BYTES + 1,
        context_limit = MAX_ORIGINAL_CONTEXT_BYTES,
        context_limit_plus_one = MAX_ORIGINAL_CONTEXT_BYTES + 1,
    )
}

pub(super) fn read_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredOperation> {
    let lineage = super::types::ExecutionLineage::new(
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
        row.get::<_, String>(4)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let generation = row.get::<_, i64>(6)?;
    let generation = u64::try_from(generation).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let operation_id = row.get::<_, String>(0)?;
    let state_id = row.get::<_, String>(5)?;
    let action_id = row.get::<_, String>(7)?;
    let action_payload = row.get::<_, Option<Vec<u8>>>(8)?;
    if row.get::<_, i64>(9)? != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let action_kind = row.get::<_, Option<String>>(10)?;
    let payload_digest = row.get::<_, String>(11)?;
    let input_digest = row.get::<_, String>(12)?;
    let catalog_digest = row.get::<_, Option<String>>(13)?;
    let catalog_raw = row.get::<_, Option<Vec<u8>>>(14)?;
    if row.get::<_, i64>(15)? != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let original_context = row.get::<_, Option<Vec<u8>>>(16)?;
    if row.get::<_, i64>(17)? != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let intent = match (action_kind, action_payload) {
        (None, None) => {
            if catalog_raw.is_some()
                || catalog_digest
                    .as_deref()
                    .is_some_and(|digest| !valid_digest(digest))
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let mut intent = OperationIntent::new(
                lineage,
                operation_id,
                state_id,
                generation,
                action_id,
                payload_digest,
                input_digest,
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
            // v4 rows may have a semantic catalog digest but cannot reconstruct the exact bytes.
            // Preserve that historical fact so recovery can explicitly block it instead of
            // treating the row as if no boundary had ever been recorded.
            intent.catalog_digest = catalog_digest;
            intent.with_optional_original_context(original_context)
        }
        (Some(action_kind), Some(action_payload))
            if !action_payload.is_empty() && action_payload.len() <= MAX_OPERATION_ACTION_BYTES =>
        {
            let calculated = crate::sha256_hex(&action_payload);
            if calculated != payload_digest {
                return Err(rusqlite::Error::InvalidQuery);
            }
            OperationIntent::new_with_action_and_catalog(
                lineage,
                operation_id,
                state_id,
                generation,
                action_id,
                action_kind,
                action_payload,
                payload_digest,
                input_digest,
                catalog_digest,
                catalog_raw,
            )
            .and_then(|intent| intent.with_optional_original_context(original_context))
        }
        _ => Err(super::types::ExecutionStoreError::InvalidOperation),
    }
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let state = OperationState::from_str(&row.get::<_, String>(18)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let result_ref = row.get::<_, Option<String>>(19)?;
    let result_digest = row.get::<_, Option<String>>(20)?;
    if result_ref.is_some() != result_digest.is_some()
        || result_ref
            .as_deref()
            .is_some_and(|value| !valid_reference(value))
        || result_digest
            .as_deref()
            .is_some_and(|value| !valid_reference(value))
        || state == OperationState::Reconciled && (result_ref.is_none() || result_digest.is_none())
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(StoredOperation {
        intent,
        state,
        result_ref,
        result_digest,
    })
}

pub(super) fn valid_transition(current: OperationState, next: OperationState) -> bool {
    matches!(
        (current, next),
        (
            OperationState::IntentRecorded,
            OperationState::MayHaveBeenDispatched
        ) | (
            OperationState::MayHaveBeenDispatched,
            OperationState::Accepted
        ) | (
            OperationState::MayHaveBeenDispatched,
            OperationState::Settled
        ) | (
            OperationState::MayHaveBeenDispatched,
            OperationState::Rejected
        ) | (
            OperationState::MayHaveBeenDispatched,
            OperationState::Unknown
        ) | (OperationState::Accepted, OperationState::Settled)
            | (OperationState::Accepted, OperationState::Rejected)
            | (OperationState::Accepted, OperationState::Unknown)
            | (OperationState::Unknown, OperationState::Reconciled)
            | (OperationState::Accepted, OperationState::Reconciled)
            | (
                OperationState::MayHaveBeenDispatched,
                OperationState::Reconciled
            )
            | (OperationState::IntentRecorded, OperationState::Reconciled)
    )
}
