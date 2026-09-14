// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::super::super::super::contract::RunSnapshot;
use super::super::super::StoreError;
use super::super::SqliteWorkflowStore;
use super::super::support::{connection, encode, read_snapshot_tx, sqlite_error};

pub(crate) fn update_run_snapshot(
    store: &SqliteWorkflowStore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
) -> Result<(), StoreError> {
    let snapshot_bytes = encode(&snapshot)?;
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let identity = transaction
        .query_row(
            "SELECT request_digest, workflow_run_id
             FROM management_submissions WHERE request_id = ?1",
            [request_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or_else(|| StoreError::new("run_not_found", "workflow submission was not found"))?;
    if identity.0 != request_digest {
        return Err(StoreError::new(
            "submission_conflict",
            "workflow submission digest does not match the reserved request",
        ));
    }
    let existing = read_snapshot_tx(&transaction, &identity.1)?
        .ok_or_else(|| StoreError::new("store_corrupt", "reserved workflow run is missing"))?;
    if existing.workflow_run_id != snapshot.workflow_run_id
        || existing.definition_digest != snapshot.definition_digest
        || existing.run_revision != snapshot.run_revision
        || existing.admission != snapshot.admission
    {
        return Err(StoreError::new(
            "snapshot_identity_conflict",
            "reserved workflow identity or target admission changed",
        ));
    }
    transaction
        .execute(
            "UPDATE management_runs SET snapshot = ?2 WHERE workflow_run_id = ?1",
            params![snapshot.workflow_run_id, snapshot_bytes],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)
}
