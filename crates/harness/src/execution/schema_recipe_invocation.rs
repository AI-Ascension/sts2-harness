// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::schema::{map_sqlite, transaction};
use super::types::ExecutionStoreError;

const MAP_INVOCATION_MIGRATION: &str = r#"
CREATE TABLE recipe_map_invocations (
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    model_execution_id TEXT NOT NULL,
    recipe_id TEXT NOT NULL CHECK (recipe_id = 'runtime.map-context'),
    recipe_revision INTEGER NOT NULL CHECK (recipe_revision = 1),
    operation TEXT NOT NULL CHECK (operation = 'map_snapshot'),
    runtime_config_digest TEXT NOT NULL CHECK (length(runtime_config_digest) = 64),
    instance_id TEXT NOT NULL,
    gateway_session_id TEXT NOT NULL,
    mcp_session_id TEXT NOT NULL,
    lease_id TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL CHECK (lease_epoch >= 0),
    state_id TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    runtime_map_profile TEXT NOT NULL CHECK (runtime_map_profile = 'runtime-map-v1'),
    schema_digest TEXT NOT NULL CHECK (length(schema_digest) = 64),
    rpc_correlation_id TEXT NOT NULL CHECK (rpc_correlation_id = '3'),
    request_digest TEXT NOT NULL CHECK (length(request_digest) = 64),
    typed_result_digest TEXT CHECK (typed_result_digest IS NULL OR length(typed_result_digest) = 64),
    owner_snapshot_digest TEXT CHECK (owner_snapshot_digest IS NULL OR length(owner_snapshot_digest) = 64),
    decision_input_digest TEXT CHECK (decision_input_digest IS NULL OR length(decision_input_digest) = 64),
    state TEXT NOT NULL CHECK (state IN ('intent_recorded', 'response_validated', 'context_validated')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, episode_id, attempt_id, trajectory_id, model_execution_id,
                 recipe_id, recipe_revision, operation),
    CHECK (
        (state = 'intent_recorded'
            AND typed_result_digest IS NULL
            AND owner_snapshot_digest IS NULL
            AND decision_input_digest IS NULL)
        OR (state = 'response_validated'
            AND typed_result_digest IS NOT NULL
            AND owner_snapshot_digest IS NULL
            AND decision_input_digest IS NULL)
        OR (state = 'context_validated'
            AND typed_result_digest IS NOT NULL
            AND owner_snapshot_digest IS NOT NULL
            AND decision_input_digest IS NOT NULL)
    )
);
"#;

/// Advances the actual v7/v8 schema to v9 without relying on the caller's stale local version.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), ExecutionStoreError> {
    let mut version = user_version(connection)?;
    if version == 7 {
        super::schema_worker::migrate(connection)?;
        version = user_version(connection)?;
    }
    match version {
        8 => migrate_v8_to_v9(connection),
        9 => Ok(()),
        _ => Err(ExecutionStoreError::Incompatible),
    }
}

fn migrate_v8_to_v9(connection: &mut Connection) -> Result<(), ExecutionStoreError> {
    let transaction = transaction(connection)?;
    transaction
        .execute_batch(MAP_INVOCATION_MIGRATION)
        .map_err(map_sqlite)?;
    transaction
        .execute_batch("PRAGMA user_version = 9")
        .map_err(map_sqlite)?;
    transaction.commit().map_err(map_sqlite)
}

fn user_version(connection: &Connection) -> Result<i32, ExecutionStoreError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
        .map_err(map_sqlite)
}
