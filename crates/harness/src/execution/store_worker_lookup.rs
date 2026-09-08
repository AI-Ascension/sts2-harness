// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::store_worker_completion::{project_completion, read_existing_completion};
use super::store_worker_queries::{read_handoff, read_handoff_by_id, worker_handoff_select};
use super::types::{
    WorkerHandoffState, WorkerLookup, WorkerTuple, valid_digest, valid_worker_uuid4,
};

impl ExecutionStore {
    /// Lookup is read-only with respect to execution. It may only project an already durable
    /// episode completion into the worker receipt; it never starts or resumes an episode.
    pub fn lookup_worker_handoff(
        &mut self,
        tuple: &WorkerTuple,
    ) -> Result<WorkerLookup, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        tuple.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let current = read_handoff_by_id(&tx, &tuple.handoff_id)?;
        let Some(current) = current else {
            let conflict = tx
                .query_row(
                    &worker_handoff_select(
                        "WHERE job_id = ?1 OR attempt_id = ?2 OR episode_id = ?3 LIMIT 1",
                    ),
                    params![tuple.job_id, tuple.attempt_id, tuple.episode_id],
                    read_handoff,
                )
                .optional()
                .map_err(schema::map_sqlite)?;
            if conflict.is_some() {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(WorkerLookup::Unknown {
                tuple: Box::new(tuple.clone()),
            });
        };
        if current.tuple != *tuple {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.terminal.is_none()
            && let Some(completion) = read_existing_completion(&tx, &tuple.episode_id)?
        {
            let receipt = receipt_from_completion(tuple, &completion)?;
            project_completion(&tx, &current, &receipt, &completion, ExecutionStore::now())?;
        }
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(WorkerLookup::Known(Box::new(
            self.worker_handoff(&tuple.handoff_id)?
                .ok_or(super::types::ExecutionStoreError::Corrupt)?,
        )))
    }

    pub fn acknowledge_worker_handoff(
        &mut self,
        tuple: &WorkerTuple,
        terminal_digest: &str,
    ) -> Result<super::types::StoredWorkerHandoff, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        tuple.validate()?;
        if !valid_digest(terminal_digest) {
            return Err(super::types::ExecutionStoreError::InvalidCompletion);
        }
        let tx = schema::transaction(&mut self.connection)?;
        let current = read_handoff_by_id(&tx, &tuple.handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Missing)?;
        if current.tuple != *tuple {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let terminal = current
            .terminal
            .as_ref()
            .ok_or(super::types::ExecutionStoreError::Conflict)?;
        let expected_digest = terminal.acknowledgment_digest()?;
        if expected_digest != terminal_digest {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let durable = read_existing_completion(&tx, &tuple.episode_id)?
            .ok_or(super::types::ExecutionStoreError::Corrupt)?;
        if receipt_from_completion(tuple, &durable)? != *terminal {
            return Err(super::types::ExecutionStoreError::Corrupt);
        }
        if current.acknowledged {
            if current.state.eq(&WorkerHandoffState::Acknowledged) {
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(current);
            }
            return Err(super::types::ExecutionStoreError::Corrupt);
        }
        let now = ExecutionStore::now();
        tx.execute(
            "UPDATE worker_handoffs SET state = 'acknowledged', acknowledged = 1,
             ack_digest = ?2, updated_at = ?3 WHERE handoff_id = ?1 AND acknowledged = 0",
            params![tuple.handoff_id, terminal_digest, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "worker_handoff",
            &tuple.handoff_id,
            WorkerHandoffState::Acknowledged.as_str(),
            Some(terminal_digest),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_handoff(&tuple.handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }

    pub fn worker_handoff(
        &self,
        handoff_id: &str,
    ) -> Result<Option<super::types::StoredWorkerHandoff>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_worker_uuid4(handoff_id) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        self.connection
            .query_row(
                &worker_handoff_select("WHERE handoff_id = ?1"),
                [handoff_id],
                read_handoff,
            )
            .optional()
            .map_err(schema::map_sqlite)
    }
}

pub(super) fn receipt_from_completion(
    tuple: &WorkerTuple,
    completion: &super::types::CompletionRecord,
) -> Result<super::types::WorkerTerminalReceipt, super::types::ExecutionStoreError> {
    super::types::worker_terminal_from_completion(tuple, completion)
}
