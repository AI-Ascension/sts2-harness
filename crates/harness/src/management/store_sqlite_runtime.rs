// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;
use serde_json::Value;

use super::super::StoreError;
use super::SqliteWorkflowStore;
use super::support::{connection, decode, encode, sqlite_error};
use crate::workflow::RuntimeSnapshot;

#[derive(Clone, Debug)]
pub(crate) struct DurableRuntimeRecord {
    pub definition_digest: String,
    pub definition: Value,
    pub snapshot: RuntimeSnapshot,
    pub cancelled: bool,
}

impl SqliteWorkflowStore {
    pub(crate) fn save_runtime(
        &self,
        run_id: &str,
        definition_digest: &str,
        definition: &Value,
        snapshot: &RuntimeSnapshot,
        cancelled: bool,
    ) -> Result<(), StoreError> {
        let definition_bytes = encode(definition)?;
        let snapshot_bytes = encode(snapshot)?;
        let mut connection = connection(self)?;
        let transaction = connection.transaction().map_err(sqlite_error)?;
        let existing = transaction
            .query_row(
                "SELECT definition_digest, definition FROM management_runtime
                 WHERE workflow_run_id = ?1",
                [run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        if let Some((stored_digest, stored_definition)) = existing
            && (stored_digest != definition_digest || stored_definition != definition_bytes)
        {
            return Err(StoreError::new(
                "runtime_definition_conflict",
                "durable runtime definition changed for an existing run",
            ));
        }
        transaction
            .execute(
                "INSERT INTO management_runtime(
                    workflow_run_id, definition_digest, definition, runtime_snapshot, cancelled
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(workflow_run_id) DO UPDATE SET
                    runtime_snapshot = excluded.runtime_snapshot,
                    cancelled = excluded.cancelled",
                rusqlite::params![
                    run_id,
                    definition_digest,
                    definition_bytes,
                    snapshot_bytes,
                    i64::from(cancelled)
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)
    }

    pub(crate) fn load_runtime(
        &self,
        run_id: &str,
    ) -> Result<Option<DurableRuntimeRecord>, StoreError> {
        let connection = connection(self)?;
        connection
            .query_row(
                "SELECT definition_digest, definition, runtime_snapshot, cancelled
                 FROM management_runtime WHERE workflow_run_id = ?1",
                [run_id],
                |row| {
                    Ok(DurableRuntimeRecord {
                        definition_digest: row.get(0)?,
                        definition: decode(&row.get::<_, Vec<u8>>(1)?)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        snapshot: decode(&row.get::<_, Vec<u8>>(2)?)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        cancelled: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()
            .map_err(sqlite_error)
    }
}
