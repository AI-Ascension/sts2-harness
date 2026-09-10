// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::super::super::super::contract::{MAX_EVENTS_PER_RUN, RunEvent, RunSnapshot};
use super::super::super::StoreError;
use super::super::SqliteWorkflowStore;
use super::super::support::{connection, encode, insert_event, sqlite_error, to_i64};
use crate::management::store::ops::validate_initial_run;

pub(crate) fn create_run(
    store: &SqliteWorkflowStore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
    initial_events: Vec<RunEvent>,
) -> Result<(), StoreError> {
    validate_initial_run(&snapshot, &initial_events)?;
    let initial_events = initial_events
        .into_iter()
        .map(|event| {
            event
                .seal_integrity()
                .map_err(|error| StoreError::new("event_integrity", error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if initial_events.len() > MAX_EVENTS_PER_RUN {
        return Err(StoreError::new(
            "event_limit",
            "initial workflow event set exceeds the supported bound",
        ));
    }
    let snapshot_bytes = encode(&snapshot)?;
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let duplicate = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM management_submissions WHERE request_id = ?1
                UNION ALL
                SELECT 1 FROM management_runs WHERE workflow_run_id = ?2
            )",
            params![request_id, snapshot.workflow_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?
        != 0;
    if duplicate {
        return Err(StoreError::new(
            "duplicate_run",
            "workflow run or submission already exists",
        ));
    }
    transaction
        .execute(
            "INSERT INTO management_submissions(request_id, request_digest, workflow_run_id)
             VALUES (?1, ?2, ?3)",
            params![request_id, request_digest, snapshot.workflow_run_id],
        )
        .map_err(sqlite_error)?;
    transaction
        .execute(
            "INSERT INTO management_runs(
                workflow_run_id, request_id, request_digest, snapshot, oldest_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                snapshot.workflow_run_id,
                request_id,
                request_digest,
                snapshot_bytes,
                to_i64(initial_events[0].sequence, "oldest event sequence")?
            ],
        )
        .map_err(sqlite_error)?;
    for event in &initial_events {
        insert_event(&transaction, event)?;
    }
    transaction.commit().map_err(sqlite_error)
}
