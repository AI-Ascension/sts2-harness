// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::super::super::super::contract::CommandRequest;
use super::super::super::StoreError;
use super::super::SqliteWorkflowStore;
use super::super::support::{connection, sqlite_error};

pub(crate) fn release_command(
    store: &SqliteWorkflowStore,
    request: &CommandRequest,
    request_digest: &str,
) -> Result<(), StoreError> {
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let changed = transaction
        .execute(
            "UPDATE management_commands SET application_in_flight = 0
             WHERE workflow_run_id = ?1 AND command_id = ?2
               AND request_digest = ?3 AND response IS NULL",
            params![request.run_id, request.command_id, request_digest],
        )
        .map_err(sqlite_error)?;
    if changed == 0 {
        let exists = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM management_commands
                    WHERE workflow_run_id = ?1 AND command_id = ?2
                )",
                params![request.run_id, request.command_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite_error)?
            != 0;
        if !exists {
            return Err(StoreError::new(
                "command_not_found",
                "command was not accepted",
            ));
        }
    }
    transaction.commit().map_err(sqlite_error)
}
