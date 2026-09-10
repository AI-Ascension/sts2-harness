// SPDX-License-Identifier: MIT

use std::sync::{MutexGuard, PoisonError};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};

use super::super::super::contract::{
    EVENT_SCHEMA_VERSION, EventClassification, EventPage, EventPayload, EventType, RunEvent,
    RunSnapshot,
};
use super::super::StoreError;
use super::SqliteWorkflowStore;

pub(super) fn connection(
    store: &SqliteWorkflowStore,
) -> Result<MutexGuard<'_, Connection>, StoreError> {
    store
        .connection
        .lock()
        .map_err(|_: PoisonError<MutexGuard<'_, Connection>>| {
            StoreError::new("store_poisoned", "workflow SQLite store lock is poisoned")
        })
}

pub(super) fn sqlite_error(error: rusqlite::Error) -> StoreError {
    StoreError::new("store_sqlite", error.to_string())
}

pub(super) fn io_error(error: std::io::Error) -> StoreError {
    StoreError::new("store_io", error.to_string())
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value).map_err(|error| StoreError::new("store_encode", error.to_string()))
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes)
        .map_err(|error| StoreError::new("store_corrupt", error.to_string()))
}

pub(super) fn to_i64(value: u64, field: &str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| {
        StoreError::new(
            "store_overflow",
            format!("{field} exceeds SQLite's supported integer range"),
        )
    })
}

pub(super) fn read_snapshot(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RunSnapshot>, StoreError> {
    let bytes = connection
        .query_row(
            "SELECT snapshot FROM management_runs WHERE workflow_run_id = ?1",
            [run_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    bytes.map(|bytes| decode(&bytes)).transpose()
}

pub(super) fn read_snapshot_tx(
    transaction: &Transaction<'_>,
    run_id: &str,
) -> Result<Option<RunSnapshot>, StoreError> {
    let bytes = transaction
        .query_row(
            "SELECT snapshot FROM management_runs WHERE workflow_run_id = ?1",
            [run_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    bytes.map(|bytes| decode(&bytes)).transpose()
}

pub(super) fn read_event(row: &Row<'_>) -> rusqlite::Result<RunEvent> {
    let bytes = row.get::<_, Vec<u8>>(0)?;
    serde_json::from_slice(&bytes).map_err(|_| rusqlite::Error::InvalidQuery)
}

pub(super) fn read_events(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<RunEvent>, StoreError> {
    let mut statement = connection
        .prepare(
            "SELECT event FROM management_events
             WHERE workflow_run_id = ?1 ORDER BY sequence",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([run_id], read_event)
        .map_err(sqlite_error)?;
    rows.map(|row| row.map_err(sqlite_error))
        .collect::<Result<Vec<_>, _>>()
}

pub(super) fn insert_event(
    transaction: &Transaction<'_>,
    event: &RunEvent,
) -> Result<(), StoreError> {
    let event = event
        .clone()
        .seal_integrity()
        .map_err(|error| StoreError::new("event_integrity", error))?;
    let bytes = encode(&event)?;
    transaction
        .execute(
            "INSERT INTO management_events(workflow_run_id, sequence, event)
             VALUES (?1, ?2, ?3)",
            params![
                event.workflow_run_id,
                to_i64(event.sequence, "event sequence")?,
                bytes
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

pub(super) fn management_event(
    snapshot: &RunSnapshot,
    sequence: u64,
    event_type: EventType,
    reason_code: String,
    classification: EventClassification,
) -> RunEvent {
    RunEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: snapshot.workflow_run_id.clone(),
        sequence,
        run_revision: snapshot.run_revision,
        event_type,
        definition_digest: snapshot.definition_digest.clone(),
        node_execution_id: "management".to_owned(),
        payload: EventPayload {
            operation_id: None,
            classification: Some(classification),
            reason_code,
        },
        integrity_digest: None,
    }
}

pub(super) fn event_page(
    run_id: &str,
    after_sequence: u64,
    limit: u64,
    events: Vec<RunEvent>,
) -> EventPage {
    let oldest_sequence = events.first().map(|event| event.sequence);
    let newest_sequence = events.last().map(|event| event.sequence);
    let bounded_limit = limit.clamp(1, super::super::super::contract::MAX_EVENTS_PER_PAGE) as usize;
    let gap = oldest_sequence
        .filter(|oldest| after_sequence.saturating_add(1) < *oldest)
        .map(|oldest| super::super::super::contract::EventGap {
            requested_after_sequence: after_sequence,
            oldest_sequence: oldest,
            newest_sequence: newest_sequence.unwrap_or(oldest),
        });
    let selected = events
        .into_iter()
        .filter(|event| event.sequence > after_sequence)
        .take(bounded_limit)
        .collect::<Vec<_>>();
    let next_after_sequence = selected
        .last()
        .map(|event| event.sequence)
        .unwrap_or(after_sequence);
    EventPage {
        schema_version: super::super::super::contract::EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        after_sequence,
        oldest_sequence,
        newest_sequence,
        next_after_sequence,
        gap,
        events: selected,
    }
}
