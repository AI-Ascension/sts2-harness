// SPDX-License-Identifier: MIT

use {super::types::ExecutionStoreError, rusqlite::Connection};

#[path = "schema_transaction.rs"]
mod schema_transaction;
#[path = "schema_worker.rs"]
mod schema_worker;

#[path = "schema_bootstrap.rs"]
mod bootstrap;

pub(crate) use schema_transaction::transaction;

pub const CURRENT_SCHEMA_VERSION: i32 = 7;

use bootstrap::MIGRATION_1;

pub(crate) fn migrate(connection: &mut Connection) -> Result<(), ExecutionStoreError> {
    let mut version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
        .map_err(map_sqlite)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(ExecutionStoreError::Incompatible);
    }
    if version == 0 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction.execute_batch(MIGRATION_1).map_err(map_sqlite)?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO store_metadata(key, value) VALUES ('recovery_contract', ?1)",
                [super::types::RECOVERY_CONTRACT_VERSION],
            )
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 1")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 1;
    }
    if version == 1 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction
            .execute("ALTER TABLE decisions ADD COLUMN result_payload BLOB", [])
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 2")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 2;
    }
    if version == 2 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction
            .execute("ALTER TABLE operations ADD COLUMN action_kind TEXT", [])
            .map_err(map_sqlite)?;
        transaction
            .execute("ALTER TABLE operations ADD COLUMN action_payload BLOB", [])
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 3")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 3;
    }
    if version == 3 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction
            .execute("ALTER TABLE operations ADD COLUMN catalog_digest TEXT", [])
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 4")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 4;
    }
    if version == 4 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction
            .execute(
                "ALTER TABLE checkpoints ADD COLUMN legal_actions_raw BLOB",
                [],
            )
            .map_err(map_sqlite)?;
        transaction
            .execute("ALTER TABLE operations ADD COLUMN catalog_raw BLOB", [])
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 5")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 5;
    }
    if version == 5 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction
            .execute(
                "ALTER TABLE operations ADD COLUMN original_context_raw BLOB",
                [],
            )
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 6")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 6;
    }
    if version == 6 {
        schema_worker::migrate(connection)?;
    }
    Ok(())
}

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
    ExecutionStoreError::Persistence(String::from("SQLite operation failed"))
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
