// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::super::schema;
use super::super::store_core::ExecutionStore;
use super::super::types::ExecutionStoreError;
use super::super::workflow_types::{
    InvocationOutcome, InvocationState, StoredWorkflowInvocation, WorkflowEvent, WorkflowEventId,
    WorkflowEventPayload, WorkflowInvocationId, WorkflowRunSnapshot,
};
use super::RunRow;
use super::helpers::{
    append_event_in_transaction, encode_projection, read_invocation, read_run, snapshot_from_row,
    to_i64, update_run_projection,
};

impl ExecutionStore {
    /// Commits the send marker after external dispatch has returned. The dispatch itself must be
    /// outside this store transaction.
    pub fn mark_workflow_invocation_sent(
        &mut self,
        invocation_id: &WorkflowInvocationId,
        expected_revision: u64,
    ) -> Result<StoredWorkflowInvocation, ExecutionStoreError> {
        self.ensure_open()?;
        invocation_id.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let current = read_invocation(&tx, invocation_id)?.ok_or(ExecutionStoreError::Missing)?;
        if current.revision != expected_revision {
            return Err(ExecutionStoreError::RevisionConflict);
        }
        if current.state != InvocationState::Reserved {
            tx.commit().map_err(schema::map_sqlite)?;
            return if current.state == InvocationState::Sent {
                Ok(current)
            } else {
                Err(ExecutionStoreError::Conflict)
            };
        }
        tx.execute(
            "UPDATE workflow_invocations
             SET state = 'sent', send_marker = 1, revision = revision + 1, updated_at = ?2
             WHERE invocation_id = ?1 AND revision = ?3 AND state = 'reserved'",
            params![
                invocation_id.as_str(),
                ExecutionStore::now(),
                to_i64(expected_revision)?
            ],
        )
        .map_err(schema::map_sqlite)?;
        let updated = read_invocation(&tx, invocation_id)?.ok_or(ExecutionStoreError::Missing)?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(updated)
    }

    /// Commits an external result and its normalized completion event atomically with the run
    /// projection. The caller supplies both CAS revisions observed before this short write.
    pub fn complete_workflow_invocation(
        &mut self,
        expected_run_revision: u64,
        expected_invocation_revision: u64,
        invocation_id: &WorkflowInvocationId,
        outcome: InvocationOutcome,
        event_id: WorkflowEventId,
    ) -> Result<(WorkflowRunSnapshot, StoredWorkflowInvocation), ExecutionStoreError> {
        self.ensure_open()?;
        invocation_id.validate()?;
        let current_invocation = self
            .workflow_invocation(invocation_id)?
            .ok_or(ExecutionStoreError::Missing)?;
        let event = WorkflowEvent::new(
            event_id,
            current_invocation.run_id.clone(),
            WorkflowEventPayload::InvocationCompleted {
                invocation_id: invocation_id.clone(),
                outcome: outcome.clone(),
            },
        )?;
        let tx = schema::transaction(&mut self.connection)?;
        let row = read_run(&tx, &event.run_id)?.ok_or(ExecutionStoreError::Missing)?;
        let invocation =
            read_invocation(&tx, invocation_id)?.ok_or(ExecutionStoreError::Missing)?;
        if row.revision != expected_run_revision {
            return Err(ExecutionStoreError::RevisionConflict);
        }
        if invocation.revision != expected_invocation_revision {
            return Err(ExecutionStoreError::RevisionConflict);
        }
        if invocation.state != InvocationState::Sent {
            return Err(ExecutionStoreError::Conflict);
        }
        let projection = append_event_in_transaction(&tx, &event, row.revision)?;
        let next_run_revision = row
            .revision
            .checked_add(1)
            .ok_or(ExecutionStoreError::InvalidWorkflowEvent)?;
        update_run_projection(
            &tx,
            event.run_id.as_str(),
            row.revision,
            next_run_revision,
            &projection,
        )?;
        let state = match outcome {
            InvocationOutcome::Accepted => InvocationState::Accepted,
            InvocationOutcome::Rejected => InvocationState::Rejected,
            InvocationOutcome::Unknown => InvocationState::Unknown,
        };
        tx.execute(
            "UPDATE workflow_invocations
             SET state = ?2, revision = revision + 1, updated_at = ?3
             WHERE invocation_id = ?1 AND revision = ?4 AND state = 'sent'",
            params![
                invocation_id.as_str(),
                state.as_str(),
                ExecutionStore::now(),
                to_i64(expected_invocation_revision)?
            ],
        )
        .map_err(schema::map_sqlite)?;
        let updated_invocation =
            read_invocation(&tx, invocation_id)?.ok_or(ExecutionStoreError::Missing)?;
        let snapshot = snapshot_from_row(RunRow {
            revision: next_run_revision,
            status: projection.status.as_str().to_string(),
            projection: encode_projection(&projection)?,
            ..row
        })?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok((snapshot, updated_invocation))
    }
}
