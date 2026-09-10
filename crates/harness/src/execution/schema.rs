// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::types::ExecutionStoreError;

#[path = "schema_support.rs"]
mod support;

pub(crate) use support::{map_sqlite, transaction};

pub const CURRENT_SCHEMA_VERSION: i32 = 8;

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
        let decisions_exist = support::table_exists(connection, "decisions")?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if decisions_exist {
            transaction
                .execute("ALTER TABLE decisions ADD COLUMN result_payload BLOB", [])
                .map_err(map_sqlite)?;
        }
        transaction
            .execute_batch("PRAGMA user_version = 2")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 2;
    }
    if version == 2 {
        let operations_exist = support::table_exists(connection, "operations")?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if operations_exist {
            transaction
                .execute("ALTER TABLE operations ADD COLUMN action_kind TEXT", [])
                .map_err(map_sqlite)?;
            transaction
                .execute("ALTER TABLE operations ADD COLUMN action_payload BLOB", [])
                .map_err(map_sqlite)?;
        }
        transaction
            .execute_batch("PRAGMA user_version = 3")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 3;
    }
    if version == 3 {
        let operations_exist = support::table_exists(connection, "operations")?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if operations_exist {
            transaction
                .execute("ALTER TABLE operations ADD COLUMN catalog_digest TEXT", [])
                .map_err(map_sqlite)?;
        }
        transaction
            .execute_batch("PRAGMA user_version = 4")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 4;
    }
    if version == 4 {
        let checkpoints_exist = support::table_exists(connection, "checkpoints")?;
        let operations_exist = support::table_exists(connection, "operations")?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if checkpoints_exist {
            transaction
                .execute(
                    "ALTER TABLE checkpoints ADD COLUMN legal_actions_raw BLOB",
                    [],
                )
                .map_err(map_sqlite)?;
        }
        if operations_exist {
            transaction
                .execute("ALTER TABLE operations ADD COLUMN catalog_raw BLOB", [])
                .map_err(map_sqlite)?;
        }
        transaction
            .execute_batch("PRAGMA user_version = 5")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 5;
    }
    if version == 5 {
        let operations_exist = support::table_exists(connection, "operations")?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        if operations_exist {
            transaction
                .execute(
                    "ALTER TABLE operations ADD COLUMN original_context BLOB",
                    [],
                )
                .map_err(map_sqlite)?;
        }
        transaction
            .execute_batch("PRAGMA user_version = 6")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 6;
    }
    if version == 6 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        transaction
            .execute_batch(super::schema_workflow::MIGRATION_2)
            .map_err(map_sqlite)?;
        transaction
            .execute_batch("PRAGMA user_version = 7")
            .map_err(map_sqlite)?;
        transaction.commit().map_err(map_sqlite)?;
        version = 7;
    }
    if version == 7 {
        super::schema_worker::migrate(connection)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
