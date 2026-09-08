// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::store_worker_queries::{
    ensure_dispatch_authority, read_handoff, read_handoff_by_id, read_job, worker_handoff_select,
};
use super::types::{JobState, WorkerAdmissionContext, WorkerHandoffState, WorkerTuple};

impl ExecutionStore {
    /// Admits one immutable worker tuple and reserves the existing durable job in the same
    /// transaction. The caller must have authenticated the peer before invoking this method.
    pub fn admit_worker_handoff(
        &mut self,
        tuple: &WorkerTuple,
        context: &WorkerAdmissionContext,
    ) -> Result<super::types::StoredWorkerHandoff, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        tuple.validate()?;
        context.validate()?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        if let Some(existing) = tx
            .query_row(
                &worker_handoff_select("WHERE handoff_id = ?1"),
                [tuple.handoff_id.as_str()],
                read_handoff,
            )
            .optional()
            .map_err(schema::map_sqlite)?
        {
            if existing.tuple != *tuple {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(existing);
        }
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
        ensure_dispatch_authority(&tx, context, tuple)?;
        let job = read_job(&tx, &tuple.job_id)?;
        if job.0 != tuple.episode_id || job.1 != tuple.payload_digest {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if job.2 != JobState::Admitted {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let episode = tx
            .query_row(
                "SELECT run_id, current_attempt_id, current_trajectory_id, state
                 FROM episodes WHERE episode_id = ?1",
                [tuple.episode_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(schema::map_sqlite)?
            .ok_or(super::types::ExecutionStoreError::Missing)?;
        if episode.0 != tuple.run_id
            || episode.1 != tuple.attempt_id
            || episode.2 != tuple.trajectory_id
            || episode.3 != "active"
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "UPDATE jobs SET state = 'claimed', claim_token = ?2, worker_id = ?3,
             updated_at = ?4 WHERE job_id = ?1 AND state = 'admitted'",
            params![tuple.job_id, tuple.handoff_id, tuple.worker_owner_id, now],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "INSERT INTO worker_handoffs(handoff_id, deployment_id, job_id, attempt_id,
             attempt_number, worker_owner_id, worker_profile_digest, run_id, episode_id,
             trajectory_id, payload_digest, worker_boot_id, watchdog_boot_id, mode_sequence,
             state, reservation_state, terminal_status, terminal_ref, checkpoint_sequence,
             result_digest, acknowledged, ack_digest, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
             'admitted', 'reserved', NULL, NULL, NULL, NULL, 0, NULL, ?15, ?15)",
            params![
                tuple.handoff_id,
                tuple.deployment_id,
                tuple.job_id,
                tuple.attempt_id,
                i64::try_from(tuple.attempt_number)
                    .map_err(|_| super::types::ExecutionStoreError::InvalidJob)?,
                tuple.worker_owner_id,
                tuple.worker_profile_digest,
                tuple.run_id,
                tuple.episode_id,
                tuple.trajectory_id,
                tuple.payload_digest,
                context.worker_boot_id,
                context.watchdog_boot_id,
                i64::try_from(context.mode_sequence)
                    .map_err(|_| super::types::ExecutionStoreError::InvalidIdentity)?,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "worker_handoff",
            &tuple.handoff_id,
            WorkerHandoffState::Admitted.as_str(),
            Some(&tuple.payload_digest),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_handoff(&tuple.handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }

    /// Marks the durable reservation as running immediately before provider/episode execution.
    pub fn mark_worker_handoff_running(
        &mut self,
        handoff_id: &str,
        context: &WorkerAdmissionContext,
    ) -> Result<super::types::StoredWorkerHandoff, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !super::types::valid_worker_uuid4(handoff_id) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        context.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let current = read_handoff_by_id(&tx, handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Missing)?;
        ensure_dispatch_authority(&tx, context, &current.tuple)?;
        if current.worker_boot_id != context.worker_boot_id
            || current.watchdog_boot_id != context.watchdog_boot_id
            || current.mode_sequence != context.mode_sequence
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.state == WorkerHandoffState::Running {
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(current);
        }
        if current.state != WorkerHandoffState::Admitted {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let now = ExecutionStore::now();
        tx.execute(
            "UPDATE worker_handoffs SET state = 'running', updated_at = ?2
             WHERE handoff_id = ?1 AND state = 'admitted'",
            params![handoff_id, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "worker_handoff",
            handoff_id,
            WorkerHandoffState::Running.as_str(),
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_handoff(handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }

    /// Retains an unresolved reservation after transport/process uncertainty. It never frees the
    /// job and never makes a later lookup eligible to start a replacement execution.
    pub fn mark_worker_handoff_unknown(
        &mut self,
        handoff_id: &str,
    ) -> Result<super::types::StoredWorkerHandoff, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !super::types::valid_worker_uuid4(handoff_id) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        let tx = schema::transaction(&mut self.connection)?;
        let current = read_handoff_by_id(&tx, handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Missing)?;
        if current.terminal.is_some() || current.state == WorkerHandoffState::Acknowledged {
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(current);
        }
        if current.state == WorkerHandoffState::Unknown {
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(current);
        }
        let now = ExecutionStore::now();
        tx.execute(
            "UPDATE worker_handoffs SET state = 'unknown', reservation_state = 'unknown',
             updated_at = ?2 WHERE handoff_id = ?1 AND terminal_status IS NULL",
            params![handoff_id, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "worker_handoff",
            handoff_id,
            WorkerHandoffState::Unknown.as_str(),
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_handoff(handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }
}
