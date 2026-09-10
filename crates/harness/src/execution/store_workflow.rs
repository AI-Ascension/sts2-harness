// SPDX-License-Identifier: MIT

mod admission;
mod helpers;
mod invocations;
mod replay;
mod validation;

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::ExecutionStore;
use super::types::ExecutionStoreError;
use super::workflow_types::{
    GameOperationId, WorkflowEvent, WorkflowEventId, WorkflowEventPayload, WorkflowInvocation,
    WorkflowRunSnapshot, WorkflowRunStart,
};
use helpers::{
    append_event_in_transaction, encode_projection, read_run, snapshot_from_projection,
    snapshot_from_row, update_run_projection,
};
use validation::validate_event_run;

#[derive(Clone, Debug)]
struct RunRow {
    run_id: String,
    workflow_id: String,
    plan_id: String,
    episode_id: String,
    revision: u64,
    status: String,
    initial_projection: Vec<u8>,
    projection: Vec<u8>,
}

impl ExecutionStore {
    /// Creates a run and commits its first journal event. The caller must perform any external
    /// effect only after this method returns successfully; no external callback is accepted while
    /// the transaction is open.
    pub fn start_workflow_run(
        &mut self,
        start: &WorkflowRunStart,
        start_event_id: WorkflowEventId,
    ) -> Result<WorkflowRunSnapshot, ExecutionStoreError> {
        self.ensure_open()?;
        start.validate()?;
        let start_event = WorkflowEvent::new(
            start_event_id,
            start.run_id.clone(),
            WorkflowEventPayload::RunStarted {
                workflow_id: start.workflow_id.clone(),
                plan_id: start.plan_id.clone(),
                episode_id: start.episode_id.clone(),
            },
        )?;
        let initial_projection = encode_projection(&start.initial_projection)?;
        let tx = schema::transaction(&mut self.connection)?;
        let plan_workflow = tx
            .query_row(
                "SELECT workflow_id FROM workflow_plans WHERE plan_id = ?1",
                [start.plan_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(schema::map_sqlite)?
            .ok_or(ExecutionStoreError::Missing)?;
        if plan_workflow != start.workflow_id.as_str() {
            return Err(ExecutionStoreError::Conflict);
        }
        let existing = read_run(&tx, &start.run_id)?;
        if let Some(existing) = existing {
            if existing.workflow_id != start.workflow_id.as_str()
                || existing.plan_id != start.plan_id.as_str()
                || existing.episode_id != start.episode_id.as_str()
                || existing.initial_projection != initial_projection
            {
                return Err(ExecutionStoreError::Conflict);
            }
            let first_event = tx
                .query_row(
                    "SELECT event_id FROM workflow_events WHERE run_id = ?1 AND sequence = 1",
                    [start.run_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .map_err(schema::map_sqlite)?;
            if first_event != start_event.event_id.as_str() {
                return Err(ExecutionStoreError::Conflict);
            }
            let snapshot = snapshot_from_row(existing)?;
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(snapshot);
        }
        tx.execute(
            "INSERT INTO workflow_runs
             (run_id, workflow_id, plan_id, episode_id, revision, status,
              initial_projection, projection, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?6, ?7, ?7)",
            params![
                start.run_id.as_str(),
                start.workflow_id.as_str(),
                start.plan_id.as_str(),
                start.episode_id.as_str(),
                start.initial_projection.status.as_str(),
                initial_projection,
                ExecutionStore::now()
            ],
        )
        .map_err(schema::map_sqlite)?;
        let projection = append_event_in_transaction(&tx, &start_event, 0)?;
        update_run_projection(&tx, start.run_id.as_str(), 0, 1, &projection)?;
        let snapshot = snapshot_from_projection(start, 1, projection);
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(snapshot)
    }

    /// Appends one normalized event using compare-and-swap on the durable run revision.
    pub fn append_workflow_event(
        &mut self,
        expected_revision: u64,
        event: &WorkflowEvent,
    ) -> Result<WorkflowRunSnapshot, ExecutionStoreError> {
        self.ensure_open()?;
        event.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let row = read_run(&tx, &event.run_id)?.ok_or(ExecutionStoreError::Missing)?;
        let revision = row.revision;
        if revision != expected_revision {
            return Err(ExecutionStoreError::RevisionConflict);
        }
        validate_event_run(&row, event)?;
        let projection = append_event_in_transaction(&tx, event, revision)?;
        let next_revision = revision
            .checked_add(1)
            .ok_or(ExecutionStoreError::InvalidWorkflowEvent)?;
        update_run_projection(
            &tx,
            event.run_id.as_str(),
            revision,
            next_revision,
            &projection,
        )?;
        let snapshot = snapshot_from_row(RunRow {
            revision: next_revision,
            status: projection.status.as_str().to_string(),
            projection: encode_projection(&projection)?,
            ..row
        })?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(snapshot)
    }

    /// Atomically records an invocation intent and its journal event. A caller may now leave the
    /// transaction boundary and invoke its external port; a missing send marker is conservative
    /// evidence that the original operation might have been sent.
    pub fn record_workflow_invocation_intent(
        &mut self,
        expected_run_revision: u64,
        invocation: &WorkflowInvocation,
        event_id: WorkflowEventId,
    ) -> Result<WorkflowRunSnapshot, ExecutionStoreError> {
        self.ensure_open()?;
        invocation.validate()?;
        let event = WorkflowEvent::new(
            event_id,
            invocation.run_id.clone(),
            WorkflowEventPayload::InvocationReserved {
                invocation_id: invocation.invocation_id.clone(),
                command_id: invocation.command_id.clone(),
                game_operation_id: invocation.game_operation_id.clone(),
            },
        )?;
        let tx = schema::transaction(&mut self.connection)?;
        let row = read_run(&tx, &invocation.run_id)?.ok_or(ExecutionStoreError::Missing)?;
        if row.revision != expected_run_revision {
            return Err(ExecutionStoreError::RevisionConflict);
        }
        if row.episode_id != invocation.episode_id.as_str()
            || row.plan_id != invocation.plan_id.as_str()
        {
            return Err(ExecutionStoreError::Conflict);
        }
        tx.execute(
            "INSERT INTO workflow_invocations
             (invocation_id, run_id, episode_id, plan_id, command_id, game_operation_id,
              payload_digest, payload, state, revision, send_marker, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'reserved', 0, 0, ?9, ?9)",
            params![
                invocation.invocation_id.as_str(),
                invocation.run_id.as_str(),
                invocation.episode_id.as_str(),
                invocation.plan_id.as_str(),
                invocation.command_id.as_str(),
                invocation
                    .game_operation_id
                    .as_ref()
                    .map(GameOperationId::as_str),
                invocation.payload_digest,
                invocation.payload,
                ExecutionStore::now()
            ],
        )
        .map_err(schema::map_sqlite)?;
        let projection = append_event_in_transaction(&tx, &event, row.revision)?;
        let next_revision = row
            .revision
            .checked_add(1)
            .ok_or(ExecutionStoreError::InvalidWorkflowEvent)?;
        update_run_projection(
            &tx,
            invocation.run_id.as_str(),
            row.revision,
            next_revision,
            &projection,
        )?;
        let snapshot = snapshot_from_row(RunRow {
            revision: next_revision,
            status: projection.status.as_str().to_string(),
            projection: encode_projection(&projection)?,
            ..row
        })?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(snapshot)
    }
}
