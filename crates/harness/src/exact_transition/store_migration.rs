// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::{BranchStoreError, DURABLE_BRANCH_SCHEMA_REVISION, DURABLE_BRANCH_SCHEMA_VERSION};

const SCHEMA: &str = include_str!("durable_branch_schema.sql");

pub(super) fn migrate(connection: &mut Connection) -> Result<(), BranchStoreError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(BranchStoreError::persistence)?;
    transaction
        .execute_batch(SCHEMA)
        .map_err(BranchStoreError::persistence)?;
    ensure_meta(&transaction)?;
    ensure_resume_claim_state(&transaction)?;
    transaction.commit().map_err(BranchStoreError::persistence)
}

fn ensure_resume_claim_state(transaction: &Transaction<'_>) -> Result<(), BranchStoreError> {
    let schema: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'branch_continuation_claims'",
            [],
            |row| row.get(0),
        )
        .map_err(BranchStoreError::persistence)?;
    if schema.contains("'resuming'") {
        return Ok(());
    }
    transaction
        .execute_batch(
            "CREATE TABLE branch_continuation_claims_v3 (
                experiment_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                operation_id TEXT NOT NULL UNIQUE REFERENCES branch_operations(operation_id),
                claim_state TEXT NOT NULL CHECK (
                    claim_state IN (
                        'prepared', 'owner_snapshotted', 'claimed', 'unknown',
                        'boundary_verified', 'resuming'
                    )
                ),
                owner_json TEXT,
                owner_digest TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (experiment_id, branch_id),
                FOREIGN KEY (experiment_id, branch_id)
                    REFERENCES durable_branches(experiment_id, branch_id),
                CHECK (
                    (owner_json IS NULL AND owner_digest IS NULL)
                    OR (owner_json IS NOT NULL AND owner_digest IS NOT NULL)
                )
            );
            INSERT INTO branch_continuation_claims_v3(
                experiment_id, branch_id, operation_id, claim_state,
                owner_json, owner_digest, created_at, updated_at
            )
            SELECT experiment_id, branch_id, operation_id, claim_state,
                   owner_json, owner_digest, created_at, updated_at
            FROM branch_continuation_claims;
            DROP TABLE branch_continuation_claims;
            ALTER TABLE branch_continuation_claims_v3
                RENAME TO branch_continuation_claims;",
        )
        .map_err(BranchStoreError::persistence)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, TransactionBehavior};

    use super::*;

    #[test]
    fn migration_rebuilds_legacy_claim_check_without_losing_rows()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE TABLE branch_schema_meta (
                schema_version TEXT PRIMARY KEY NOT NULL,
                migration_revision INTEGER NOT NULL
            );
            INSERT INTO branch_schema_meta VALUES ('ascension.durable-branch/v1', 2);
            CREATE TABLE durable_branches (
                experiment_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                PRIMARY KEY (experiment_id, branch_id)
            );
            CREATE TABLE branch_operations (
                operation_id TEXT PRIMARY KEY NOT NULL
            );
            CREATE TABLE branch_continuation_claims (
                experiment_id TEXT NOT NULL,
                branch_id TEXT NOT NULL,
                operation_id TEXT NOT NULL UNIQUE REFERENCES branch_operations(operation_id),
                claim_state TEXT NOT NULL CHECK (
                    claim_state IN ('prepared', 'owner_snapshotted', 'claimed', 'unknown', 'boundary_verified')
                ),
                owner_json TEXT,
                owner_digest TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (experiment_id, branch_id),
                FOREIGN KEY (experiment_id, branch_id)
                    REFERENCES durable_branches(experiment_id, branch_id),
                CHECK (
                    (owner_json IS NULL AND owner_digest IS NULL)
                    OR (owner_json IS NOT NULL AND owner_digest IS NOT NULL)
                )
            );
            INSERT INTO durable_branches VALUES ('experiment:test', 'branch:test');
            INSERT INTO branch_operations VALUES ('operation:test');
            INSERT INTO branch_continuation_claims VALUES (
                'experiment:test', 'branch:test', 'operation:test',
                'boundary_verified', '{\"owner\":true}', 'digest', 1, 1
            );",
        )?;

        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_resume_claim_state(&transaction)?;
        transaction.commit()?;

        let claim: (String, String, String) = connection.query_row(
            "SELECT claim_state, owner_json, owner_digest
             FROM branch_continuation_claims
             WHERE experiment_id = 'experiment:test' AND branch_id = 'branch:test'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(
            claim,
            (
                String::from("boundary_verified"),
                String::from("{\"owner\":true}"),
                String::from("digest")
            )
        );
        let schema: String = connection.query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'branch_continuation_claims'",
            [],
            |row| row.get(0),
        )?;
        assert!(schema.contains("'resuming'"));
        Ok(())
    }
}
