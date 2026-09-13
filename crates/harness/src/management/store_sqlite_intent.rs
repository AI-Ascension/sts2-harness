// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::super::super::super::contract::{EventClassification, EventType, PendingOperation};
use super::super::super::StoreError;
use super::super::SqliteWorkflowStore;
use super::super::support::{
    connection, encode, insert_event, management_event as sqlite_event, read_snapshot_tx,
    sqlite_error,
};

pub(crate) fn record_operation_intent(
    store: &SqliteWorkflowStore,
    run_id: &str,
    expected_revision: u64,
    pending: PendingOperation,
) -> Result<(), StoreError> {
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let mut snapshot = read_snapshot_tx(&transaction, run_id)?
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    if snapshot.run_revision != expected_revision {
        return Err(StoreError::new(
            "stale_revision",
            "operation intent revision does not match the durable run",
        ));
    }
    if let Some(existing) = &snapshot.pending_operation {
        if existing == &pending {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(());
        }
        return Err(StoreError::new(
            "operation_identity_conflict",
            "a different pending operation is already durable",
        ));
    }
    snapshot.pending_operation = Some(pending);
    let sequence = super::next_sequence(&transaction, run_id)?;
    let event = sqlite_event(
        &snapshot,
        sequence,
        EventType::OperationIntent,
        "live_operation_intent".to_owned(),
        EventClassification::Accepted,
    );
    insert_event(&transaction, &event)?;
    transaction
        .execute(
            "UPDATE management_runs SET snapshot = ?2 WHERE workflow_run_id = ?1",
            params![run_id, encode(&snapshot)?],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)
}
