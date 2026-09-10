// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Transaction};

use super::super::types::ExecutionStoreError;

pub(crate) fn map_sqlite(error: rusqlite::Error) -> ExecutionStoreError {
    if matches!(error, rusqlite::Error::InvalidQuery) {
        return ExecutionStoreError::Corrupt;
    }
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    ) {
        return ExecutionStoreError::Busy;
    }
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref failure, _)
            if matches!(failure.code, rusqlite::ErrorCode::ConstraintViolation)
    ) {
        return ExecutionStoreError::Conflict;
    }
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    ) {
        return ExecutionStoreError::Corrupt;
    }
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref failure, _)
            if matches!(failure.code, rusqlite::ErrorCode::DiskFull)
    ) {
        return ExecutionStoreError::StorageFull;
    }
    ExecutionStoreError::Persistence(String::from("SQLite operation failed"))
}

pub(crate) fn map_transaction(error: rusqlite::Error) -> ExecutionStoreError {
    map_sqlite(error)
}

pub(crate) fn transaction<'a>(
    connection: &'a mut Connection,
) -> Result<Transaction<'a>, ExecutionStoreError> {
    connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_transaction)
}

pub(super) fn table_exists(
    connection: &Connection,
    table: &str,
) -> Result<bool, ExecutionStoreError> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(map_sqlite)
}
