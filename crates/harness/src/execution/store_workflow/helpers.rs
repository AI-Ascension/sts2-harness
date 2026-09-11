// SPDX-License-Identifier: MIT

use std::convert::TryFrom;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::super::schema;
use super::super::store_core::ExecutionStore;
use super::super::types::ExecutionStoreError;
use super::super::workflow_types::{
    InvocationOutcome, InvocationState, RunProjection, RunStatus, StoredWorkflowInvocation,
    WorkflowEvent, WorkflowEventPayload, WorkflowRunId, WorkflowRunSnapshot, WorkflowRunStart,
    digest_bytes,
};
use super::RunRow;

const MAX_EVENT_SEQUENCE: u64 = 9_007_199_254_740_991;

pub(super) fn read_run(
    connection: &Connection,
    run_id: &WorkflowRunId,
) -> Result<Option<RunRow>, ExecutionStoreError> {
    connection
        .query_row(
            "SELECT run_id, workflow_id, plan_id, episode_id, revision, status,
                    initial_projection, projection
             FROM workflow_runs WHERE run_id = ?1",
            [run_id.as_str()],
            |row| {
                Ok(RunRow {
                    run_id: row.get(0)?,
                    workflow_id: row.get(1)?,
                    plan_id: row.get(2)?,
                    episode_id: row.get(3)?,
                    revision: to_u64(row.get::<_, i64>(4)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    status: row.get(5)?,
                    initial_projection: row.get(6)?,
                    projection: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| match error {
            rusqlite::Error::InvalidQuery => ExecutionStoreError::Corrupt,
            other => schema::map_sqlite(other),
        })
}

pub(super) fn read_invocation(
    connection: &Connection,
    invocation_id: &super::super::workflow_types::WorkflowInvocationId,
) -> Result<Option<StoredWorkflowInvocation>, ExecutionStoreError> {
    connection
        .query_row(
            "SELECT run_id, episode_id, plan_id, command_id, game_operation_id,
                    payload_digest, state, revision, send_marker
             FROM workflow_invocations WHERE invocation_id = ?1",
            [invocation_id.as_str()],
            |row| {
                let run_id =
                    super::super::workflow_types::WorkflowRunId::new(row.get::<_, String>(0)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let episode_id =
                    super::super::workflow_types::WorkflowEpisodeId::new(row.get::<_, String>(1)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let plan_id =
                    super::super::workflow_types::WorkflowPlanId::new(row.get::<_, String>(2)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let command_id =
                    super::super::workflow_types::WorkflowCommandId::new(row.get::<_, String>(3)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let game_operation_id = row
                    .get::<_, Option<String>>(4)?
                    .map(|value| {
                        super::super::workflow_types::GameOperationId::new(value)
                            .map_err(|_| rusqlite::Error::InvalidQuery)
                    })
                    .transpose()?;
                let state = InvocationState::from_str(&row.get::<_, String>(6)?)
                    .ok_or(rusqlite::Error::InvalidQuery)?;
                Ok(StoredWorkflowInvocation {
                    invocation_id: invocation_id.clone(),
                    run_id,
                    episode_id,
                    plan_id,
                    command_id,
                    game_operation_id,
                    payload_digest: row.get(5)?,
                    state,
                    revision: to_u64(row.get::<_, i64>(7)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    send_marker: row.get::<_, i64>(8)? != 0,
                })
            },
        )
        .optional()
        .map_err(|error| match error {
            rusqlite::Error::InvalidQuery => ExecutionStoreError::Corrupt,
            other => schema::map_sqlite(other),
        })
}

pub(super) fn append_event_in_transaction(
    tx: &Transaction<'_>,
    event: &WorkflowEvent,
    revision: u64,
) -> Result<RunProjection, ExecutionStoreError> {
    let row = read_run(tx, &event.run_id)?.ok_or(ExecutionStoreError::Missing)?;
    if row.revision != revision {
        return Err(ExecutionStoreError::RevisionConflict);
    }
    let projection = apply_payload(decode_projection(&row.projection)?, &event.payload)?;
    let sequence = revision
        .checked_add(1)
        .filter(|value| *value <= MAX_EVENT_SEQUENCE)
        .ok_or(ExecutionStoreError::InvalidWorkflowEvent)?;
    let payload = serde_json::to_vec(&event.payload).map_err(|_| {
        ExecutionStoreError::Persistence(String::from("workflow event encoding failed"))
    })?;
    tx.execute(
        "INSERT INTO workflow_events
         (event_id, run_id, sequence, kind, payload, payload_digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.event_id.as_str(),
            event.run_id.as_str(),
            to_i64(sequence)?,
            event.payload.kind(),
            payload,
            digest_bytes(&serde_json::to_vec(&event.payload).map_err(|_| {
                ExecutionStoreError::Persistence(String::from("workflow event encoding failed"))
            })?),
            ExecutionStore::now()
        ],
    )
    .map_err(schema::map_sqlite)?;
    Ok(projection)
}

pub(super) fn update_run_projection(
    tx: &Transaction<'_>,
    run_id: &str,
    expected_revision: u64,
    next_revision: u64,
    projection: &RunProjection,
) -> Result<(), ExecutionStoreError> {
    let projection_bytes = encode_projection(projection)?;
    let changed = tx
        .execute(
            "UPDATE workflow_runs SET revision = ?2, status = ?3, projection = ?4,
             updated_at = ?5 WHERE run_id = ?1 AND revision = ?6",
            params![
                run_id,
                to_i64(next_revision)?,
                projection.status.as_str(),
                projection_bytes,
                ExecutionStore::now(),
                to_i64(expected_revision)?
            ],
        )
        .map_err(schema::map_sqlite)?;
    if changed != 1 {
        return Err(ExecutionStoreError::RevisionConflict);
    }
    Ok(())
}

pub(super) fn apply_payload(
    mut projection: RunProjection,
    payload: &WorkflowEventPayload,
) -> Result<RunProjection, ExecutionStoreError> {
    payload.validate()?;
    match payload {
        WorkflowEventPayload::RunStarted { .. } => {
            if projection.status != RunStatus::Created {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            projection.status = RunStatus::Running;
            projection.increment("run_started")?;
        }
        WorkflowEventPayload::CursorAdvanced {
            cursor,
            stack,
            counters,
        } => {
            if projection.status != RunStatus::Running || *cursor <= projection.cursor {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            if projection
                .counters
                .iter()
                .any(|(name, previous)| counters.get(name).copied().unwrap_or(0) < *previous)
            {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            projection.cursor = *cursor;
            projection.stack = stack.clone();
            projection.counters = counters.clone();
            projection.increment("cursor_advanced")?;
        }
        WorkflowEventPayload::InvocationReserved { .. } => {
            if projection.status != RunStatus::Running {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            projection.increment("invocations_reserved")?;
        }
        WorkflowEventPayload::InvocationCompleted { outcome, .. } => {
            if projection.status != RunStatus::Running {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            let counter = match outcome {
                InvocationOutcome::Accepted => "invocations_accepted",
                InvocationOutcome::Rejected => "invocations_rejected",
                InvocationOutcome::Unknown => "invocations_unknown",
            };
            projection.increment(counter)?;
        }
        WorkflowEventPayload::RunCompleted => {
            if projection.status != RunStatus::Running {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            projection.status = RunStatus::Completed;
            projection.increment("run_completed")?;
        }
        WorkflowEventPayload::RunFailed => {
            if projection.status != RunStatus::Running {
                return Err(ExecutionStoreError::InvalidWorkflowEvent);
            }
            projection.status = RunStatus::Failed;
            projection.increment("run_failed")?;
        }
    }
    projection.validate()?;
    Ok(projection)
}

pub(super) fn snapshot_from_row(row: RunRow) -> Result<WorkflowRunSnapshot, ExecutionStoreError> {
    let run_id = WorkflowRunId::new(row.run_id)?;
    let workflow_id = super::super::workflow_types::WorkflowDefinitionId::new(row.workflow_id)?;
    let plan_id = super::super::workflow_types::WorkflowPlanId::new(row.plan_id)?;
    let episode_id = super::super::workflow_types::WorkflowEpisodeId::new(row.episode_id)?;
    let projection = decode_projection(&row.projection)?;
    if RunStatus::from_str(&row.status).is_none() || projection.status.as_str() != row.status {
        return Err(ExecutionStoreError::Corrupt);
    }
    Ok(WorkflowRunSnapshot {
        run_id,
        workflow_id,
        plan_id,
        episode_id,
        revision: row.revision,
        projection,
    })
}

pub(super) fn snapshot_from_projection(
    start: &WorkflowRunStart,
    revision: u64,
    projection: RunProjection,
) -> WorkflowRunSnapshot {
    WorkflowRunSnapshot {
        run_id: start.run_id.clone(),
        workflow_id: start.workflow_id.clone(),
        plan_id: start.plan_id.clone(),
        episode_id: start.episode_id.clone(),
        revision,
        projection,
    }
}

pub(super) fn encode_projection(
    projection: &RunProjection,
) -> Result<Vec<u8>, ExecutionStoreError> {
    projection.validate()?;
    serde_json::to_vec(projection).map_err(|_| {
        ExecutionStoreError::Persistence(String::from("workflow projection encoding failed"))
    })
}

pub(super) fn decode_projection(bytes: &[u8]) -> Result<RunProjection, ExecutionStoreError> {
    let projection: RunProjection =
        serde_json::from_slice(bytes).map_err(|_| ExecutionStoreError::Corrupt)?;
    projection.validate()?;
    Ok(projection)
}

pub(super) fn to_i64(value: u64) -> Result<i64, ExecutionStoreError> {
    i64::try_from(value).map_err(|_| ExecutionStoreError::InvalidWorkflowEvent)
}

pub(super) fn to_u64(value: i64) -> Result<u64, ExecutionStoreError> {
    u64::try_from(value).map_err(|_| ExecutionStoreError::Corrupt)
}
