// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Transaction};

use super::types::ExecutionStoreError;

pub const CURRENT_SCHEMA_VERSION: i32 = 2;

const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS store_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    seed TEXT NOT NULL,
    build_digest TEXT NOT NULL,
    state_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    provider_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS episodes (
    episode_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    current_attempt_id TEXT NOT NULL,
    current_trajectory_id TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS attempts (
    attempt_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    trajectory_id TEXT NOT NULL,
    parent_attempt_id TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    seed TEXT NOT NULL,
    build_digest TEXT NOT NULL,
    state_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    provider_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(parent_attempt_id) REFERENCES attempts(attempt_id)
);
CREATE TABLE IF NOT EXISTS trajectories (
    trajectory_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    provenance_ref TEXT,
    provenance_digest TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS checkpoints (
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    sequence INTEGER NOT NULL,
    state_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    seed TEXT NOT NULL,
    build_digest TEXT NOT NULL,
    state_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    provider_digest TEXT NOT NULL,
    observation BLOB NOT NULL,
    legal_actions_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(episode_id, attempt_id, sequence)
);
CREATE TABLE IF NOT EXISTS operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    state_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    action_id TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    input_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    result_ref TEXT,
    result_digest TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS operations_episode_state
    ON operations(episode_id, state);
CREATE TABLE IF NOT EXISTS decisions (
    execution_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    input_fingerprint TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    result_ref TEXT,
    result_digest TEXT,
    provider_reservation_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_reservations (
    reservation_id TEXT PRIMARY KEY NOT NULL,
    execution_id TEXT NOT NULL UNIQUE REFERENCES decisions(execution_id),
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    provider_execution_id TEXT NOT NULL UNIQUE,
    reserved_units INTEGER NOT NULL,
    actual_units INTEGER,
    state TEXT NOT NULL,
    failure_class TEXT,
    result_ref TEXT,
    result_digest TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS completions (
    episode_id TEXT PRIMARY KEY NOT NULL REFERENCES episodes(episode_id),
    run_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    status TEXT NOT NULL,
    terminal_ref TEXT NOT NULL,
    checkpoint_sequence INTEGER NOT NULL,
    result_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    payload_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    claim_token TEXT,
    worker_id TEXT,
    result_ref TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS execution_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    state TEXT NOT NULL,
    detail_digest TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS execution_events_entity
    ON execution_events(entity_kind, entity_id, event_id);
"#;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_v1_migrates_decisions_to_result_aware_v2() -> Result<(), Box<dyn std::error::Error>> {
        let mut connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE TABLE decisions (
                execution_id TEXT PRIMARY KEY NOT NULL,
                run_id TEXT NOT NULL,
                episode_id TEXT NOT NULL,
                attempt_id TEXT NOT NULL,
                trajectory_id TEXT NOT NULL,
                input_fingerprint TEXT NOT NULL,
                model_revision TEXT NOT NULL,
                config_digest TEXT NOT NULL,
                state TEXT NOT NULL,
                result_ref TEXT,
                result_digest TEXT,
                provider_reservation_id TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            PRAGMA user_version = 1;",
        )?;

        migrate(&mut connection)?;

        let version =
            connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?;
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        let payload_column = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'result_payload'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(payload_column, 1);
        Ok(())
    }

    #[test]
    fn future_schema_is_rejected_before_any_migration() -> Result<(), Box<dyn std::error::Error>> {
        let mut connection = Connection::open_in_memory()?;
        connection.execute_batch("PRAGMA user_version = 99;")?;
        assert_eq!(
            migrate(&mut connection),
            Err(ExecutionStoreError::Incompatible)
        );
        Ok(())
    }
}
