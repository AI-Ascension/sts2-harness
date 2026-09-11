// SPDX-License-Identifier: MIT

use super::super::schema;
use super::super::store_core::ExecutionStore;
use super::super::types::ExecutionStoreError;
use super::super::workflow_types::digest_bytes;
use super::super::workflow_types::{
    StoredWorkflowInvocation, WorkflowEventPayload, WorkflowInvocationId, WorkflowRunId,
    WorkflowRunSnapshot,
};
use super::RunRow;
use super::helpers::{
    apply_payload, decode_projection, encode_projection, read_invocation, read_run,
    snapshot_from_row, to_u64,
};

impl ExecutionStore {
    pub fn workflow_run(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<Option<WorkflowRunSnapshot>, ExecutionStoreError> {
        self.ensure_open()?;
        run_id.validate()?;
        read_run(&self.connection, run_id)?
            .map(snapshot_from_row)
            .transpose()
    }

    pub fn workflow_invocation(
        &self,
        invocation_id: &WorkflowInvocationId,
    ) -> Result<Option<StoredWorkflowInvocation>, ExecutionStoreError> {
        self.ensure_open()?;
        invocation_id.validate()?;
        read_invocation(&self.connection, invocation_id)
    }

    /// Rebuilds the projection solely from the immutable initial projection and ordered events.
    /// A mismatch with the stored projection is corruption, not a reason to recreate state.
    pub fn replay_workflow_run(
        &self,
        run_id: &WorkflowRunId,
    ) -> Result<WorkflowRunSnapshot, ExecutionStoreError> {
        self.ensure_open()?;
        run_id.validate()?;
        let row = read_run(&self.connection, run_id)?.ok_or(ExecutionStoreError::Missing)?;
        let mut projection = decode_projection(&row.initial_projection)?;
        let mut expected_sequence = 1_u64;
        let mut statement = self
            .connection
            .prepare(
                "SELECT sequence, kind, payload, payload_digest
                 FROM workflow_events WHERE run_id = ?1 ORDER BY sequence",
            )
            .map_err(schema::map_sqlite)?;
        let mut rows = statement
            .query([run_id.as_str()])
            .map_err(schema::map_sqlite)?;
        while let Some(event) = rows.next().map_err(schema::map_sqlite)? {
            let sequence = to_u64(event.get::<_, i64>(0).map_err(schema::map_sqlite)?)?;
            if sequence != expected_sequence {
                return Err(ExecutionStoreError::Corrupt);
            }
            let kind = event.get::<_, String>(1).map_err(schema::map_sqlite)?;
            let payload = event.get::<_, Vec<u8>>(2).map_err(schema::map_sqlite)?;
            let digest = event.get::<_, String>(3).map_err(schema::map_sqlite)?;
            if digest != digest_bytes(&payload) {
                return Err(ExecutionStoreError::Corrupt);
            }
            let payload: WorkflowEventPayload =
                serde_json::from_slice(&payload).map_err(|_| ExecutionStoreError::Corrupt)?;
            if payload.kind() != kind {
                return Err(ExecutionStoreError::Corrupt);
            }
            projection = apply_payload(projection, &payload)?;
            expected_sequence = expected_sequence
                .checked_add(1)
                .ok_or(ExecutionStoreError::Corrupt)?;
        }
        let event_count = expected_sequence.saturating_sub(1);
        if event_count != row.revision || projection != decode_projection(&row.projection)? {
            return Err(ExecutionStoreError::Corrupt);
        }
        snapshot_from_row(RunRow {
            projection: encode_projection(&projection)?,
            ..row
        })
    }
}
