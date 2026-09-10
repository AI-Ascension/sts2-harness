// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::super::super::contract::{
    CommandOutcome, CommandRequest, CommandResponse, EventClassification, EventPage, EventType,
    MAX_EVENTS_PER_RUN, RunSnapshot,
};
use super::super::{CommandAcceptance, CommandApplication, StoreError, SubmissionLookup};
use super::SqliteWorkflowStore;
use super::support::{
    connection, decode, encode, event_page, insert_event, management_event as sqlite_event,
    read_events, read_snapshot, read_snapshot_tx, sqlite_error,
};

#[path = "store_sqlite_export.rs"]
mod export;
pub(super) use export::export;
#[path = "store_sqlite_create.rs"]
mod create;
pub(crate) use create::create_run;
#[path = "store_sqlite_release.rs"]
mod release;
pub(crate) use release::release_command;

pub(super) fn lookup_submission(
    store: &SqliteWorkflowStore,
    request_id: &str,
    request_digest: &str,
) -> Result<SubmissionLookup, StoreError> {
    let connection = connection(store)?;
    let row = connection
        .query_row(
            "SELECT request_digest, workflow_run_id FROM management_submissions
             WHERE request_id = ?1",
            [request_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((stored_digest, run_id)) = row else {
        return Ok(SubmissionLookup::Missing);
    };
    if stored_digest != request_digest {
        return Ok(SubmissionLookup::Conflict);
    }
    read_snapshot(&connection, &run_id)?
        .map(|snapshot| SubmissionLookup::Existing(Box::new(snapshot)))
        .ok_or_else(|| {
            StoreError::new(
                "store_corrupt",
                "submission index points to a missing workflow run",
            )
        })
}

pub(super) fn get_run(
    store: &SqliteWorkflowStore,
    run_id: &str,
) -> Result<Option<RunSnapshot>, StoreError> {
    let connection = connection(store)?;
    read_snapshot(&connection, run_id)
}

pub(super) fn events(
    store: &SqliteWorkflowStore,
    run_id: &str,
    after_sequence: u64,
    limit: u64,
) -> Result<EventPage, StoreError> {
    let connection = connection(store)?;
    if read_snapshot(&connection, run_id)?.is_none() {
        return Err(StoreError::new(
            "run_not_found",
            "workflow run was not found",
        ));
    }
    Ok(event_page(
        run_id,
        after_sequence,
        limit,
        read_events(&connection, run_id)?,
    ))
}

pub(super) fn accept_command(
    store: &SqliteWorkflowStore,
    request: &CommandRequest,
    request_digest: &str,
) -> Result<CommandAcceptance, StoreError> {
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let snapshot = read_snapshot_tx(&transaction, &request.run_id)?
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    let existing = transaction
        .query_row(
            "SELECT request_digest, application_in_flight, response
             FROM management_commands WHERE workflow_run_id = ?1 AND command_id = ?2",
            params![request.run_id, request.command_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?;
    if let Some((stored_digest, in_flight, response)) = existing {
        transaction.commit().map_err(sqlite_error)?;
        if stored_digest != request_digest {
            return Ok(CommandAcceptance::Conflict);
        }
        return Ok(CommandAcceptance::Existing {
            snapshot,
            response: response.map(|bytes| decode(&bytes)).transpose()?,
            application_in_flight: in_flight,
        });
    }
    if request.expected_revision != snapshot.run_revision {
        return Err(StoreError::new(
            "stale_revision",
            "command expected revision does not match the durable run revision",
        ));
    }
    let sequence = next_sequence(&transaction, &request.run_id)?;
    let event = sqlite_event(
        &snapshot,
        sequence,
        EventType::CommandRequested,
        request.kind.reason_code().to_owned(),
        EventClassification::Accepted,
    );
    insert_event(&transaction, &event)?;
    transaction
        .execute(
            "INSERT INTO management_commands(
                workflow_run_id, command_id, request_digest, application_in_flight, response
             ) VALUES (?1, ?2, ?3, 1, NULL)",
            params![request.run_id, request.command_id, request_digest],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)?;
    Ok(CommandAcceptance::New {
        snapshot,
        requested_sequence: sequence,
    })
}

pub(super) fn apply_command(
    store: &SqliteWorkflowStore,
    request: &CommandRequest,
    request_digest: &str,
    application: CommandApplication,
) -> Result<CommandResponse, StoreError> {
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let snapshot = read_snapshot_tx(&transaction, &request.run_id)?
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    let (stored_digest, response) = transaction
        .query_row(
            "SELECT request_digest, response FROM management_commands
             WHERE workflow_run_id = ?1 AND command_id = ?2",
            params![request.run_id, request.command_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or_else(|| StoreError::new("command_not_found", "command was not accepted"))?;
    if stored_digest != request_digest {
        return Err(StoreError::new(
            "command_conflict",
            "command ID was reused with a different payload",
        ));
    }
    if let Some(response) = response {
        transaction.commit().map_err(sqlite_error)?;
        return decode(&response);
    }
    let expected_revision = snapshot
        .run_revision
        .checked_add(1)
        .ok_or_else(|| StoreError::new("revision_overflow", "run revision overflowed"))?;
    if application.snapshot.workflow_run_id != request.run_id
        || application.snapshot.run_revision != expected_revision
    {
        return Err(StoreError::new(
            "port_revision_mismatch",
            "execution port returned an unexpected run revision",
        ));
    }
    let sequence = next_sequence(&transaction, &request.run_id)?;
    let event = sqlite_event(
        &application.snapshot,
        sequence,
        EventType::CommandApplied,
        application.reason_code,
        classification_for_outcome(&application.outcome),
    );
    insert_event(&transaction, &event)?;
    let response = CommandResponse {
        schema_version: super::super::super::contract::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: request.command_id.clone(),
        workflow_run_id: request.run_id.clone(),
        outcome: application.outcome,
        run_revision: application.snapshot.run_revision,
        sequence: Some(sequence),
    };
    transaction
        .execute(
            "UPDATE management_runs SET snapshot = ?2 WHERE workflow_run_id = ?1",
            params![request.run_id, encode(&application.snapshot)?],
        )
        .map_err(sqlite_error)?;
    transaction
        .execute(
            "UPDATE management_commands SET application_in_flight = 0, response = ?3
             WHERE workflow_run_id = ?1 AND command_id = ?2",
            params![request.run_id, request.command_id, encode(&response)?],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)?;
    Ok(response)
}

fn classification_for_outcome(outcome: &CommandOutcome) -> EventClassification {
    match outcome {
        CommandOutcome::Accepted | CommandOutcome::Applied | CommandOutcome::Duplicate => {
            EventClassification::Settled
        }
        CommandOutcome::Pending => EventClassification::Unknown,
    }
}

fn next_sequence(transaction: &rusqlite::Transaction<'_>, run_id: &str) -> Result<u64, StoreError> {
    let sequence = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM management_events
             WHERE workflow_run_id = ?1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?;
    let sequence = u64::try_from(sequence)
        .map_err(|_| StoreError::new("sequence_overflow", "event sequence overflowed"))?;
    if sequence == 0 || sequence > MAX_EVENTS_PER_RUN as u64 {
        return Err(StoreError::new(
            "event_limit",
            "workflow event retention limit has been reached",
        ));
    }
    Ok(sequence)
}
