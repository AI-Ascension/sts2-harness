// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Transaction};

use super::{BranchStoreError, DURABLE_BRANCH_SCHEMA_REVISION, DURABLE_BRANCH_SCHEMA_VERSION};

const SCHEMA: &str = include_str!("durable_branch_schema.sql");

pub(super) fn migrate(connection: &mut Connection) -> Result<(), BranchStoreError> {
    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(BranchStoreError::persistence)?;
    if version > DURABLE_BRANCH_SCHEMA_REVISION {
        return Err(BranchStoreError::UnsupportedSchema);
    }
    let transaction = connection
        .transaction()
        .map_err(BranchStoreError::persistence)?;
    transaction
        .execute_batch(SCHEMA)
        .map_err(BranchStoreError::persistence)?;
    ensure_meta(&transaction)?;
    transaction
        .execute_batch(&format!(
            "PRAGMA user_version = {DURABLE_BRANCH_SCHEMA_REVISION};"
        ))
        .map_err(BranchStoreError::persistence)?;
    transaction.commit().map_err(BranchStoreError::persistence)
}

fn ensure_meta(transaction: &Transaction<'_>) -> Result<(), BranchStoreError> {
    let mut versions = transaction
        .prepare("SELECT schema_version, migration_revision FROM branch_schema_meta")
        .map_err(BranchStoreError::persistence)?;
    let rows = versions
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(BranchStoreError::persistence)?;
    for row in rows {
        let (version, revision) = row.map_err(BranchStoreError::persistence)?;
        if version != DURABLE_BRANCH_SCHEMA_VERSION || revision > DURABLE_BRANCH_SCHEMA_REVISION {
            return Err(BranchStoreError::UnsupportedSchema);
        }
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO branch_schema_meta(schema_version, migration_revision)
             VALUES (?1, ?2)",
            rusqlite::params![
                DURABLE_BRANCH_SCHEMA_VERSION,
                DURABLE_BRANCH_SCHEMA_REVISION
            ],
        )
        .map_err(BranchStoreError::persistence)?;
    transaction
        .execute(
            "UPDATE branch_schema_meta SET migration_revision = ?2
             WHERE schema_version = ?1 AND migration_revision < ?2",
            rusqlite::params![
                DURABLE_BRANCH_SCHEMA_VERSION,
                DURABLE_BRANCH_SCHEMA_REVISION
            ],
        )
        .map_err(BranchStoreError::persistence)?;
    let stored: i64 = transaction
        .query_row(
            "SELECT migration_revision FROM branch_schema_meta WHERE schema_version = ?1",
            [DURABLE_BRANCH_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .map_err(BranchStoreError::persistence)?;
    if stored != DURABLE_BRANCH_SCHEMA_REVISION {
        return Err(BranchStoreError::UnsupportedSchema);
    }
    Ok(())
}
