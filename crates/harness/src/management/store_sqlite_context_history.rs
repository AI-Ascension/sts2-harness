// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, Transaction, params};

use super::super::super::context_binding_history::{
    MAX_BINDING_HISTORY_BYTES, MAX_BINDING_HISTORY_PER_RUN, invalid,
};
use super::super::super::{CommandRequest, CommandResponse, RecordedContextBinding, RunSnapshot};
use super::super::StoreError;
use super::SqliteWorkflowStore;
use super::support::{connection, decode, encode, read_snapshot, sqlite_error};

pub(super) fn check_capacity(store: &SqliteWorkflowStore, run_id: &str) -> Result<(), StoreError> {
    let connection = connection(store)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM management_context_binding_history WHERE workflow_run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if count >= MAX_BINDING_HISTORY_PER_RUN as i64 {
        return Err(StoreError::new(
            "context_history_limit",
            "context binding history is full",
        ));
    }
    Ok(())
}

pub(super) fn insert(
    transaction: &Transaction<'_>,
    request: &CommandRequest,
    before: &RunSnapshot,
    after: &RunSnapshot,
    record: &RecordedContextBinding,
) -> Result<(), StoreError> {
    record.validate()?;
    record
        .binding
        .validate(Some(before))
        .map_err(|_| invalid())?;
    if record.command_id != request.command_id
        || record.binding.workflow_run_id != request.run_id
        || record.run_revision != after.run_revision
        || after.definition_digest != before.definition_digest
        || request.expected_revision != before.run_revision
        || !matches!(request.kind, super::super::super::CommandKind::Step)
    {
        return Err(invalid());
    }
    let count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM management_context_binding_history WHERE workflow_run_id = ?1",
            [&request.run_id],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    let bytes = encode(record)?;
    if count >= MAX_BINDING_HISTORY_PER_RUN as i64 || bytes.len() > MAX_BINDING_HISTORY_BYTES {
        return Err(StoreError::new(
            "context_history_limit",
            "context binding history is full",
        ));
    }
    transaction
        .execute(
            "INSERT INTO management_context_binding_history(
            workflow_run_id, node_execution_id, command_id, record
         ) VALUES (?1, ?2, ?3, ?4)",
            params![
                request.run_id,
                record.binding.node_execution_id,
                request.command_id,
                bytes
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

pub(super) fn verify_replay(
    transaction: &Transaction<'_>,
    request: &CommandRequest,
    response: &CommandResponse,
    record: &RecordedContextBinding,
) -> Result<(), StoreError> {
    record.validate()?;
    if record.command_id != request.command_id
        || record.binding.workflow_run_id != request.run_id
        || record.run_revision != response.run_revision
        || response.command_id != request.command_id
        || response.workflow_run_id != request.run_id
    {
        return Err(invalid());
    }
    let bytes = encode(record)?;
    if bytes.len() > MAX_BINDING_HISTORY_BYTES {
        return Err(invalid());
    }
    let equal: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM management_context_binding_history
         WHERE workflow_run_id = ?1 AND node_execution_id = ?2 AND command_id = ?3 AND record = ?4)",
        params![record.binding.workflow_run_id, record.binding.node_execution_id,
                record.command_id, bytes],
        |row| row.get(0),
    ).map_err(sqlite_error)?;
    if !equal {
        return Err(StoreError::new(
            "context_history_conflict",
            "command replay changed historical context evidence",
        ));
    }
    Ok(())
}

pub(super) fn read(
    store: &SqliteWorkflowStore,
    run_id: &str,
    node_execution_id: &str,
) -> Result<Option<RecordedContextBinding>, StoreError> {
    let connection = connection(store)?;
    let snapshot = read_snapshot(&connection, run_id)?
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    let row = connection
        .query_row(
            "SELECT length(history.record),
                CASE WHEN length(history.record) <= 16384 THEN history.record ELSE NULL END,
                history.command_id,
                CASE WHEN length(command.response) <= 16384 THEN command.response ELSE NULL END
         FROM management_context_binding_history AS history
         LEFT JOIN management_commands AS command
           ON command.workflow_run_id = history.workflow_run_id
          AND command.command_id = history.command_id
         WHERE history.workflow_run_id = ?1 AND history.node_execution_id = ?2",
            params![run_id, node_execution_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((length, bytes, command_id, response)) = row else {
        return Ok(None);
    };
    if length > MAX_BINDING_HISTORY_BYTES as i64 {
        return Err(invalid());
    }
    let record: RecordedContextBinding =
        decode(&bytes.ok_or_else(invalid)?).map_err(|_| invalid())?;
    let response: CommandResponse =
        decode(&response.ok_or_else(invalid)?).map_err(|_| invalid())?;
    validate_read(
        &record,
        &snapshot,
        node_execution_id,
        &command_id,
        &response,
    )?;
    Ok(Some(record))
}

fn validate_read(
    record: &RecordedContextBinding,
    snapshot: &RunSnapshot,
    node_execution_id: &str,
    command_id: &str,
    response: &CommandResponse,
) -> Result<(), StoreError> {
    record.validate()?;
    if response.schema_version != super::super::super::MANAGEMENT_SCHEMA_VERSION
        || response.sequence.is_none()
        || record.binding.workflow_run_id != snapshot.workflow_run_id
        || record.binding.definition_digest != snapshot.definition_digest
        || record.binding.node_execution_id != node_execution_id
        || record.command_id != command_id
        || record.command_id != response.command_id
        || record.run_revision != response.run_revision
        || response.workflow_run_id != snapshot.workflow_run_id
        || record.run_revision > snapshot.run_revision
    {
        return Err(invalid());
    }
    Ok(())
}
