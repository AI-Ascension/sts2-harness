// SPDX-License-Identifier: MIT

use super::lifetime_durable::LIFETIME_AAD;
use super::lifetime_scope::MAX_LIFETIME_STATE_BYTES;
use super::state::ControlEvent;
use super::state::MAX_CONTROL_EVENTS;
use super::store::decrypt_with_key;
use super::store_receipts::owner_receipt_aad;
use super::store_render_sources::source_aad;
use super::store_types::{
    CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION, DurableControlStoreError, LEGACY_STORE_SCHEMA,
    MAX_CONTEXT_SOURCE_BYTES, MAX_EVENT_BYTES, MAX_EVENTS, MAX_JOURNAL_BYTES,
    MAX_OWNER_RECEIPT_BYTES, STORE_SCHEMA,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RECEIPT_ENVELOPE_OVERHEAD: usize = 24 + 16;
const MAX_V1_RECEIPT_SCAN_BYTES: usize =
    MAX_CONTROL_EVENTS as usize * (MAX_OWNER_RECEIPT_BYTES + MAX_RECEIPT_ENVELOPE_OVERHEAD);

pub(super) fn ensure_schema(
    connection: &Connection,
    key: &[u8; 32],
    run_id: &str,
) -> Result<(), DurableControlStoreError> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
    let has_metadata = transaction
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'context_control_meta'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .is_some();
    if !has_metadata {
        if database_has_run_data(&transaction)? {
            return Err(DurableControlStoreError::MigrationRequired);
        }
        transaction
            .execute_batch(SCHEMA)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO context_control_meta(key, value) VALUES ('schema', ?1)",
                [STORE_SCHEMA],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO context_control_meta(key, value) VALUES ('schema_version', ?1)",
                [CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION.to_string()],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        return Ok(());
    }

    // Read the marker only after taking SQLite's immediate writer lock. A concurrent open that
    // began as v1 may have completed the migration while this connection was waiting.
    let schema = transaction
        .query_row(
            "SELECT value FROM context_control_meta WHERE key = 'schema'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let version = transaction
        .query_row(
            "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .map(|value| value.parse::<i64>())
        .transpose()
        .map_err(|_| DurableControlStoreError::Incompatible)?;

    if schema.is_none() && version.is_none() {
        if database_has_run_data(&transaction)? {
            return Err(DurableControlStoreError::Incompatible);
        }
        transaction
            .execute_batch(SCHEMA)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO context_control_meta(key, value) VALUES ('schema', ?1)",
                [STORE_SCHEMA],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO context_control_meta(key, value) VALUES ('schema_version', ?1)",
                [CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION.to_string()],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        return Ok(());
    }
    let (Some(schema), Some(version)) = (schema, version) else {
        return Err(DurableControlStoreError::Incompatible);
    };

    if schema == LEGACY_STORE_SCHEMA && version == 1 {
        // Authenticate the exact v1 rows while holding the same immediate writer lock used for
        // the schema change. This prevents an independently opened v1 writer from replacing an
        // encrypted record between authentication and migration.
        authenticate_v1_before_migration(&transaction, key, run_id)?;
        transaction
            .execute_batch(SCHEMA)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let schema_updated = transaction
            .execute(
                "UPDATE context_control_meta SET value = ?1 WHERE key = 'schema' AND value = ?2",
                params![STORE_SCHEMA, LEGACY_STORE_SCHEMA],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let version_updated = transaction
            .execute(
                "UPDATE context_control_meta SET value = ?1 WHERE key = 'schema_version' AND value = '1'",
                [CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION.to_string()],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if schema_updated != 1 || version_updated != 1 {
            return Err(DurableControlStoreError::Incompatible);
        }
    } else if schema != STORE_SCHEMA || version != CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION {
        return Err(DurableControlStoreError::Incompatible);
    } else {
        // Repair only absent idempotent DDL under a known current schema marker.
        transaction
            .execute_batch(SCHEMA)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
    }
    transaction
        .commit()
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    Ok(())
}

/// Authenticate every encrypted v1 record for the requested existing run before `ensure_schema`
/// changes the database-global marker. Other runs can use independent keys; their bytes remain
/// untouched and are not claimed as authenticated by this migration. Work is bounded per run.

include!("store_schema_migration.rs");
include!("store_schema_tables.rs");
