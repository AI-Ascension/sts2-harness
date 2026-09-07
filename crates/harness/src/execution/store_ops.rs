// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event, ensure_current_lineage};
use super::store_operation_queries::{read_operation, valid_transition};
use super::types::{OperationIntent, OperationState, StoredOperation, valid_reference};

const MAX_OPERATIONS: i64 = 4_096;

impl ExecutionStore {
    /// Commits an immutable operation identity before a transport can receive it. Repeating an
    /// identical intent returns the retained record; changing any immutable field is a conflict.
    pub fn record_operation_intent(
        &mut self,
        intent: &OperationIntent,
    ) -> Result<StoredOperation, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        intent.lineage.validate()?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let existing = tx
            .query_row(
                "SELECT operation_id, run_id, episode_id, attempt_id, trajectory_id, state_id,
                 generation, action_id, action_kind, action_payload, payload_digest, input_digest,
                 catalog_digest, state, result_ref, result_digest
                 FROM operations WHERE operation_id = ?1",
                [intent.operation_id.as_str()],
                read_operation,
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(existing) = existing {
            if existing.intent != *intent {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(existing);
        }
        ensure_current_lineage(&tx, &intent.lineage)?;
        let count = tx
            .query_row("SELECT COUNT(*) FROM operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(schema::map_sqlite)?;
        if count >= MAX_OPERATIONS {
            return Err(super::types::ExecutionStoreError::Capacity);
        }
        tx.execute(
            "INSERT INTO operations(operation_id, run_id, episode_id, attempt_id, trajectory_id,
             state_id, generation, action_id, action_kind, action_payload, payload_digest,
             input_digest, catalog_digest, state, result_ref, result_digest, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL, NULL, ?15, ?15)",
            params![
                intent.operation_id,
                intent.lineage.run_id,
                intent.lineage.episode_id,
                intent.lineage.attempt_id,
                intent.lineage.trajectory_id,
                intent.state_id,
                i64::try_from(intent.generation)
                    .map_err(|_| { super::types::ExecutionStoreError::InvalidOperation })?,
                intent.action_id,
                intent.action_kind,
                intent.action_payload,
                intent.payload_digest,
                intent.input_digest,
                intent.catalog_digest,
                OperationState::IntentRecorded.as_str(),
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "operation",
            &intent.operation_id,
            OperationState::IntentRecorded.as_str(),
            Some(&intent.payload_digest),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.operation(&intent.operation_id)
    }

    /// Commits MAY_HAVE_BEEN_DISPATCHED immediately before handing work to MCP/gateway.
    pub fn mark_operation_dispatched(
        &mut self,
        operation_id: &str,
        payload_digest: &str,
    ) -> Result<StoredOperation, super::types::ExecutionStoreError> {
        self.transition_operation(
            operation_id,
            payload_digest,
            OperationState::MayHaveBeenDispatched,
            None,
            None,
        )
    }

    pub fn record_operation_result(
        &mut self,
        operation_id: &str,
        payload_digest: &str,
        state: OperationState,
        result_ref: Option<&str>,
        result_digest: Option<&str>,
    ) -> Result<StoredOperation, super::types::ExecutionStoreError> {
        if !matches!(
            state,
            OperationState::Accepted
                | OperationState::Settled
                | OperationState::Rejected
                | OperationState::Unknown
        ) {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        if result_ref.is_some_and(|value| !valid_reference(value))
            || result_digest.is_some_and(|value| !valid_reference(value))
            || result_ref.is_some() != result_digest.is_some()
        {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        self.transition_operation(
            operation_id,
            payload_digest,
            state,
            result_ref,
            result_digest,
        )
    }

    /// Resolves the original operation identity after an authoritative lookup. This method has
    /// no transport call and cannot dispatch a replacement operation.
    pub fn reconcile_operation(
        &mut self,
        operation_id: &str,
        payload_digest: &str,
        resolved_state: OperationState,
        result_ref: &str,
        result_digest: &str,
    ) -> Result<StoredOperation, super::types::ExecutionStoreError> {
        if !matches!(
            resolved_state,
            OperationState::Settled | OperationState::Rejected
        ) {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        if !valid_reference(result_ref) || !valid_reference(result_digest) {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        self.transition_operation(
            operation_id,
            payload_digest,
            OperationState::Reconciled,
            Some(result_ref),
            Some(result_digest),
        )
    }

    pub fn operation(
        &self,
        operation_id: &str,
    ) -> Result<StoredOperation, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
            .query_row(
                "SELECT operation_id, run_id, episode_id, attempt_id, trajectory_id, state_id,
                 generation, action_id, action_kind, action_payload, payload_digest, input_digest,
                 catalog_digest, state, result_ref, result_digest
                 FROM operations WHERE operation_id = ?1",
                [operation_id],
                read_operation,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })
    }

    pub fn pending_operations(
        &self,
        episode_id: &str,
    ) -> Result<Vec<StoredOperation>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT operation_id, run_id, episode_id, attempt_id, trajectory_id, state_id,
                 generation, action_id, action_kind, action_payload, payload_digest, input_digest,
                 catalog_digest, state, result_ref, result_digest
                 FROM operations WHERE episode_id = ?1 AND state IN
                 ('intent_recorded', 'may_have_been_dispatched', 'accepted', 'unknown')
                 ORDER BY created_at, operation_id",
            )
            .map_err(schema::map_sqlite)?;
        let rows = statement
            .query_map([episode_id], read_operation)
            .map_err(schema::map_sqlite)?;
        rows.map(|row| row.map_err(schema::map_sqlite))
            .collect::<Result<Vec<_>, _>>()
    }

    fn transition_operation(
        &mut self,
        operation_id: &str,
        payload_digest: &str,
        next: OperationState,
        result_ref: Option<&str>,
        result_digest: Option<&str>,
    ) -> Result<StoredOperation, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_reference(operation_id) || !valid_reference(payload_digest) {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT operation_id, run_id, episode_id, attempt_id, trajectory_id, state_id,
                 generation,
                 action_id, action_kind, action_payload, payload_digest, input_digest,
                 catalog_digest, state, result_ref, result_digest
                 FROM operations WHERE operation_id = ?1",
                [operation_id],
                read_operation,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if let Some(result_ref) = result_ref
            && current
                .result_ref
                .as_deref()
                .is_some_and(|old| old != result_ref)
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if let Some(result_digest) = result_digest
            && current
                .result_digest
                .as_deref()
                .is_some_and(|old| old != result_digest)
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.intent.payload_digest != payload_digest {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.state == next {
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(current);
        }
        if current.state == OperationState::Reconciled {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if !valid_transition(current.state, next) {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "UPDATE operations SET state = ?2, result_ref = COALESCE(?3, result_ref),
             result_digest = COALESCE(?4, result_digest), updated_at = ?5
             WHERE operation_id = ?1",
            params![operation_id, next.as_str(), result_ref, result_digest, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "operation",
            operation_id,
            next.as_str(),
            result_digest,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.operation(operation_id)
    }
}
