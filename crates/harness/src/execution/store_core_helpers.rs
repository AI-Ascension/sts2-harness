// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::schema;
use super::types::{
    AttemptKind, ExecutionFingerprint, ExecutionLineage, ExecutionStoreConfig, ExecutionStoreError,
    valid_digest,
};

pub(crate) fn attempt_fingerprint(
    tx: &rusqlite::Transaction<'_>,
    attempt_id: &str,
) -> Result<ExecutionFingerprint, ExecutionStoreError> {
    tx.query_row(
        "SELECT seed, build_digest, state_digest, config_digest, provider_digest
        FROM attempts WHERE attempt_id = ?1",
        [attempt_id],
        |row| {
            ExecutionFingerprint::new(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => ExecutionStoreError::Missing,
        rusqlite::Error::InvalidQuery => ExecutionStoreError::Corrupt,
        other => schema::map_sqlite(other),
    })
}

pub(crate) fn insert_attempt(
    tx: &rusqlite::Transaction<'_>,
    lineage: &ExecutionLineage,
    fingerprint: &ExecutionFingerprint,
    kind: AttemptKind,
    parent_attempt_id: Option<&str>,
    now: i64,
) -> Result<(), ExecutionStoreError> {
    tx.execute(
        "INSERT INTO attempts(attempt_id, run_id, episode_id, trajectory_id, parent_attempt_id,
         kind, state, seed, build_digest, state_digest, config_digest, provider_digest,
         created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        params![
            lineage.attempt_id,
            lineage.run_id,
            lineage.episode_id,
            lineage.trajectory_id,
            parent_attempt_id,
            kind.as_str(),
            fingerprint.seed,
            fingerprint.build_digest,
            fingerprint.state_digest,
            fingerprint.config_digest,
            fingerprint.provider_digest,
            now
        ],
    )
    .map_err(schema::map_sqlite)?;
    Ok(())
}

/// Verifies that a record belongs to the currently active attempt for an episode. SQLite foreign
/// keys alone cannot express the current-attempt tuple, and accepting an old tuple would let a
/// restarted worker append new work to a superseded attempt.
pub(crate) fn ensure_current_lineage(
    tx: &rusqlite::Transaction<'_>,
    lineage: &ExecutionLineage,
) -> Result<(), ExecutionStoreError> {
    lineage.validate()?;
    let current = tx
        .query_row(
            "SELECT run_id, current_attempt_id, current_trajectory_id, state
             FROM episodes WHERE episode_id = ?1",
            [lineage.episode_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ExecutionStoreError::Missing,
            other => schema::map_sqlite(other),
        })?;
    if current.0 != lineage.run_id
        || current.1 != lineage.attempt_id
        || current.2 != lineage.trajectory_id
        || current.3 != "active"
    {
        return Err(ExecutionStoreError::Conflict);
    }
    Ok(())
}

pub(crate) fn append_event(
    tx: &rusqlite::Transaction<'_>,
    entity_kind: &str,
    entity_id: &str,
    state: &str,
    detail_digest: Option<&str>,
    now: i64,
) -> Result<(), ExecutionStoreError> {
    tx.execute(
        "INSERT INTO execution_events(entity_kind, entity_id, state, detail_digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![entity_kind, entity_id, state, detail_digest, now],
    )
    .map_err(schema::map_sqlite)?;
    Ok(())
}

pub(crate) fn configure(
    connection: &mut Connection,
    config: &ExecutionStoreConfig,
) -> Result<(), ExecutionStoreError> {
    if rusqlite::version_number() < 3_051_001 {
        return Err(ExecutionStoreError::Incompatible);
    }
    let timeout = config.busy_timeout.as_millis();
    let sql = format!(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = {timeout};"
    );
    connection.execute_batch(&sql).map_err(schema::map_sqlite)
}

pub(crate) fn verify_durable_pragmas(
    connection: &Connection,
    path: &Path,
) -> Result<(), ExecutionStoreError> {
    if path == Path::new(":memory:") {
        return Ok(());
    }
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .map_err(schema::map_sqlite)?;
    let synchronous = connection
        .query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))
        .map_err(schema::map_sqlite)?;
    let foreign_keys = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map_err(schema::map_sqlite)?;
    if !journal_mode.eq_ignore_ascii_case("wal") || synchronous != 2 || foreign_keys != 1 {
        return Err(ExecutionStoreError::InvalidConfiguration);
    }
    Ok(())
}

pub(crate) fn verify_contract(
    connection: &Connection,
    config: &ExecutionStoreConfig,
    read_only: bool,
) -> Result<(), ExecutionStoreError> {
    let contract = connection
        .query_row(
            "SELECT value FROM store_metadata WHERE key = 'recovery_contract'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(schema::map_sqlite)?;
    if contract.as_deref() != Some(super::types::RECOVERY_CONTRACT_VERSION) {
        return Err(ExecutionStoreError::Incompatible);
    }
    let stored_digest = connection
        .query_row(
            "SELECT value FROM store_metadata WHERE key = 'recovery_schema_digest'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(schema::map_sqlite)?;
    if let Some(expected) = config.recovery_schema_digest.as_deref() {
        if !valid_digest(expected) {
            return Err(ExecutionStoreError::InvalidFingerprint);
        }
        if stored_digest.as_deref().is_some_and(|old| old != expected) {
            return Err(ExecutionStoreError::Incompatible);
        }
        if stored_digest.is_none() {
            if read_only {
                return Err(ExecutionStoreError::Incompatible);
            }
            connection
                .execute(
                    "INSERT INTO store_metadata(key, value) VALUES ('recovery_schema_digest', ?1)",
                    [expected],
                )
                .map_err(schema::map_sqlite)?;
        }
    }
    let workflow_schema = connection
        .query_row(
            "SELECT value FROM store_metadata WHERE key = 'workflow_schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(schema::map_sqlite)?;
    let workflow_schema_version = workflow_schema
        .as_deref()
        .and_then(|value| value.parse::<i32>().ok());
    if workflow_schema_version != Some(schema::WORKFLOW_SCHEMA_VERSION) {
        return Err(ExecutionStoreError::Incompatible);
    }
    let workflow_contract = connection
        .query_row(
            "SELECT value FROM store_metadata WHERE key = 'workflow_contract'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(schema::map_sqlite)?;
    if workflow_contract.as_deref() != Some("workflow-v1") {
        return Err(ExecutionStoreError::Incompatible);
    }
    Ok(())
}

pub(crate) fn create_parent(path: &Path) -> Result<(), ExecutionStoreError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || parent.as_os_str() == ":memory:" {
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|_| {
        ExecutionStoreError::Persistence(String::from("cannot create execution state directory"))
    })
}

pub(crate) fn has_store_schema(connection: &Connection) -> Result<bool, ExecutionStoreError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'store_metadata')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(schema::map_sqlite)
}
