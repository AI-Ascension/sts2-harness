// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::{ExecutionStoreError, map_sqlite};

const WORKER_MIGRATION: &str = r#"
CREATE TABLE worker_control (
    control_id INTEGER PRIMARY KEY NOT NULL CHECK (control_id = 1),
    deployment_id TEXT NOT NULL,
    worker_owner_id TEXT NOT NULL,
    worker_profile_digest TEXT NOT NULL,
    worker_boot_id TEXT NOT NULL,
    watchdog_boot_id TEXT,
    mode TEXT NOT NULL,
    mode_sequence INTEGER NOT NULL,
    generation INTEGER NOT NULL,
    authenticated INTEGER NOT NULL,
    admitting INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE worker_control_boots (
    boot_id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    generation INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE worker_handoffs (
    handoff_id TEXT PRIMARY KEY NOT NULL,
    deployment_id TEXT NOT NULL,
    job_id TEXT NOT NULL UNIQUE REFERENCES jobs(job_id),
    attempt_id TEXT NOT NULL UNIQUE REFERENCES attempts(attempt_id),
    attempt_number INTEGER NOT NULL,
    worker_owner_id TEXT NOT NULL,
    worker_profile_digest TEXT NOT NULL,
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL UNIQUE REFERENCES episodes(episode_id),
    trajectory_id TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    worker_boot_id TEXT NOT NULL,
    watchdog_boot_id TEXT NOT NULL,
    mode_sequence INTEGER NOT NULL,
    state TEXT NOT NULL,
    reservation_state TEXT NOT NULL,
    terminal_status TEXT,
    terminal_ref TEXT,
    checkpoint_sequence INTEGER,
    result_digest TEXT,
    terminal_record BLOB,
    acknowledged INTEGER NOT NULL,
    ack_digest TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX worker_handoffs_state
    ON worker_handoffs(state, reservation_state);
"#;

pub(crate) fn migrate(connection: &mut Connection) -> Result<(), ExecutionStoreError> {
    // Version 6 cannot own any worker-v7 object: the DDL and version advance
    // commit together. A collision is incompatible state, not an interrupted
    // migration to silently bless with IF NOT EXISTS.
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_sqlite)?;
    transaction
        .execute_batch(WORKER_MIGRATION)
        .map_err(map_sqlite)?;
    transaction
        .execute_batch("PRAGMA user_version = 7")
        .map_err(map_sqlite)?;
    transaction.commit().map_err(map_sqlite)
}
