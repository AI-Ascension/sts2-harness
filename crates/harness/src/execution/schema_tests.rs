// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::{CURRENT_SCHEMA_VERSION, migrate};
use crate::execution::ExecutionStoreError;

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
        CREATE TABLE operations (operation_id TEXT PRIMARY KEY NOT NULL);
        CREATE TABLE checkpoints (legal_actions_digest TEXT NOT NULL);
        PRAGMA user_version = 1;",
    )?;

    migrate(&mut connection)?;

    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?;
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    let payload_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'result_payload'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(payload_column, 1);
    let action_kind_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name = 'action_kind'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(action_kind_column, 1);
    let action_payload_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name = 'action_payload'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(action_payload_column, 1);
    let catalog_digest_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name = 'catalog_digest'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(catalog_digest_column, 1);
    let checkpoint_catalog_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('checkpoints') WHERE name = 'legal_actions_raw'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(checkpoint_catalog_column, 1);
    let operation_catalog_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name = 'catalog_raw'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(operation_catalog_column, 1);
    let original_context_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('operations') WHERE name = 'original_context_raw'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(original_context_column, 1);
    Ok(())
}

#[test]
fn v5_migration_rolls_back_both_columns_when_the_second_alter_fails()
-> Result<(), Box<dyn std::error::Error>> {
    let mut connection = Connection::open_in_memory()?;
    connection.execute_batch(
        "CREATE TABLE checkpoints (legal_actions_digest TEXT NOT NULL);
         CREATE TABLE operations (catalog_raw BLOB);
         PRAGMA user_version = 4;",
    )?;

    assert!(migrate(&mut connection).is_err());
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))?;
    assert_eq!(version, 4);
    let checkpoint_catalog_column = connection.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('checkpoints') WHERE name = 'legal_actions_raw'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(checkpoint_catalog_column, 0);
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
