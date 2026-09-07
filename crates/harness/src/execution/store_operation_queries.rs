// SPDX-License-Identifier: MIT

use super::types::{OperationIntent, OperationState, StoredOperation, valid_reference};

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
    let intent = OperationIntent::new(
        lineage,
        row.get::<_, String>(0)?,
        row.get::<_, String>(5)?,
        generation,
        row.get::<_, String>(7)?,
        row.get::<_, String>(8)?,
        row.get::<_, String>(9)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let state = OperationState::from_str(&row.get::<_, String>(10)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let result_ref = row.get::<_, Option<String>>(11)?;
    let result_digest = row.get::<_, Option<String>>(12)?;
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
