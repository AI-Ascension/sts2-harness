// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::store_worker_lookup::receipt_from_completion;
use super::store_worker_queries::{read_handoff_by_id, read_job};
use super::types::{
    CompletionRecord, CompletionStatus, JobState, WorkerCompletionStatus, WorkerHandoffState,
    WorkerTerminalReceipt, WorkerTuple,
};
use crate::worker_handoff::TerminalRecord;

impl ExecutionStore {
    pub fn record_worker_completion(
        &mut self,
        receipt: &WorkerTerminalReceipt,
    ) -> Result<super::types::StoredWorkerHandoff, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        receipt.validate()?;
        let tx = schema::transaction(&mut self.connection)?;
        let current = read_handoff_by_id(&tx, &receipt.tuple.handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Missing)?;
        if current.tuple != receipt.tuple {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if let Some(existing) = current.terminal.as_ref() {
            if existing == receipt {
                let durable = read_existing_completion(&tx, &receipt.tuple.episode_id)?;
                if durable.as_ref().is_none_or(|completion| {
                    !receipt_from_completion(&receipt.tuple, completion)
                        .is_ok_and(|projected| projected == *receipt)
                }) {
                    return Err(super::types::ExecutionStoreError::Corrupt);
                }
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(current);
            }
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let now = ExecutionStore::now();
        if let Some(completion) = read_existing_completion(&tx, &receipt.tuple.episode_id)? {
            project_completion(&tx, &current, receipt, &completion, now)?;
        } else {
            commit_new_completion(&tx, &current, receipt, now)?;
        }
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_handoff(&receipt.tuple.handoff_id)?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }

    /// Records a canonical terminal after durable completion. Acknowledgment is a separate call.
    pub fn record_worker_terminal(
        &mut self,
        tuple: &WorkerTuple,
        terminal: &TerminalRecord,
    ) -> Result<super::types::StoredWorkerHandoff, super::types::ExecutionStoreError> {
        let receipt = WorkerTerminalReceipt::from_terminal(tuple.clone(), terminal)?;
        self.record_worker_completion(&receipt)
    }
}

pub(super) fn project_completion(
    tx: &rusqlite::Transaction<'_>,
    current: &super::types::StoredWorkerHandoff,
    receipt: &WorkerTerminalReceipt,
    completion: &CompletionRecord,
    now: i64,
) -> Result<(), super::types::ExecutionStoreError> {
    if receipt_from_completion(&receipt.tuple, completion)? != *receipt {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    if current.terminal.is_some() {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    let episode = tx
        .query_row(
            "SELECT run_id, current_attempt_id, current_trajectory_id, state
             FROM episodes WHERE episode_id = ?1",
            [receipt.tuple.episode_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(schema::map_sqlite)?;
    if episode.0 != receipt.tuple.run_id
        || episode.1 != receipt.tuple.attempt_id
        || episode.2 != receipt.tuple.trajectory_id
        || episode.3 != completion.status.as_str()
    {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    let job = read_job(tx, &receipt.tuple.job_id)?;
    if job.0 != receipt.tuple.episode_id
        || job.1 != receipt.tuple.payload_digest
        || job.2 != JobState::Claimed
        || job.3.as_deref() != Some(receipt.tuple.handoff_id.as_str())
    {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    tx.execute(
        "UPDATE worker_handoffs SET state = 'terminal', reservation_state = 'reserved',
         terminal_status = ?2,
         terminal_ref = ?3, checkpoint_sequence = ?4, result_digest = ?5,
         terminal_record = ?6, updated_at = ?7
         WHERE handoff_id = ?1 AND terminal_status IS NULL",
        params![
            receipt.tuple.handoff_id,
            receipt.status.as_str(),
            receipt.terminal_ref,
            i64::try_from(receipt.checkpoint_sequence)
                .map_err(|_| super::types::ExecutionStoreError::InvalidCompletion)?,
            receipt.result_digest,
            receipt.canonical_bytes(),
            now
        ],
    )
    .map_err(schema::map_sqlite)?;
    let job_state = match receipt.status {
        WorkerCompletionStatus::Completed => "completed",
        WorkerCompletionStatus::Failed => "failed",
    };
    let updated = tx
        .execute(
            "UPDATE jobs SET state = ?2, result_ref = ?3, updated_at = ?4
         WHERE job_id = ?1 AND claim_token = ?5",
            params![
                receipt.tuple.job_id,
                job_state,
                receipt.terminal_ref,
                now,
                receipt.tuple.handoff_id
            ],
        )
        .map_err(schema::map_sqlite)?;
    if updated != 1 {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    append_event(
        tx,
        "worker_handoff",
        &receipt.tuple.handoff_id,
        WorkerHandoffState::Terminal.as_str(),
        Some(&receipt.result_digest),
        now,
    )?;
    Ok(())
}

fn commit_new_completion(
    tx: &rusqlite::Transaction<'_>,
    current: &super::types::StoredWorkerHandoff,
    receipt: &WorkerTerminalReceipt,
    now: i64,
) -> Result<(), super::types::ExecutionStoreError> {
    if current.state == WorkerHandoffState::Acknowledged {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    let current_episode = tx
        .query_row(
            "SELECT run_id, current_attempt_id, current_trajectory_id, state
             FROM episodes WHERE episode_id = ?1",
            [receipt.tuple.episode_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(schema::map_sqlite)?;
    if current_episode.0 != receipt.tuple.run_id
        || current_episode.1 != receipt.tuple.attempt_id
        || current_episode.2 != receipt.tuple.trajectory_id
        || current_episode.3 != "active"
    {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    let job = read_job(tx, &receipt.tuple.job_id)?;
    if job.0 != receipt.tuple.episode_id
        || job.1 != receipt.tuple.payload_digest
        || job.3.as_deref() != Some(receipt.tuple.handoff_id.as_str())
    {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    if job.2 != JobState::Claimed {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    let checkpoint_exists = tx
        .query_row(
            "SELECT COUNT(*) FROM checkpoints WHERE episode_id = ?1 AND attempt_id = ?2
             AND sequence = ?3",
            params![
                receipt.tuple.episode_id,
                receipt.tuple.attempt_id,
                i64::try_from(receipt.checkpoint_sequence)
                    .map_err(|_| super::types::ExecutionStoreError::InvalidCompletion)?
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(schema::map_sqlite)?;
    if checkpoint_exists != 1 {
        return Err(super::types::ExecutionStoreError::InvalidCompletion);
    }
    let unresolved_operations = unresolved_count(
        tx,
        "SELECT COUNT(*) FROM operations WHERE episode_id = ?1 AND state IN
         ('intent_recorded', 'may_have_been_dispatched', 'accepted', 'unknown')",
        &receipt.tuple.episode_id,
    )?;
    let unresolved_decisions = unresolved_count(
        tx,
        "SELECT COUNT(*) FROM decisions WHERE episode_id = ?1 AND state IN ('pending', 'unknown')",
        &receipt.tuple.episode_id,
    )?;
    let unresolved_reservations = unresolved_count(
        tx,
        "SELECT COUNT(*) FROM provider_reservations WHERE episode_id = ?1
         AND state IN ('reserved', 'unknown')",
        &receipt.tuple.episode_id,
    )?;
    if unresolved_operations != 0 || unresolved_decisions != 0 || unresolved_reservations != 0 {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    let status = match receipt.status {
        WorkerCompletionStatus::Completed => CompletionStatus::Completed,
        WorkerCompletionStatus::Failed => CompletionStatus::Failed,
    };
    tx.execute(
        "INSERT INTO completions(episode_id, run_id, attempt_id, trajectory_id, status,
         terminal_ref, checkpoint_sequence, result_digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            receipt.tuple.episode_id,
            receipt.tuple.run_id,
            receipt.tuple.attempt_id,
            receipt.tuple.trajectory_id,
            status.as_str(),
            receipt.terminal_ref,
            i64::try_from(receipt.checkpoint_sequence)
                .map_err(|_| super::types::ExecutionStoreError::InvalidCompletion)?,
            receipt.result_digest,
            now
        ],
    )
    .map_err(schema::map_sqlite)?;
    tx.execute(
        "UPDATE attempts SET state = ?2, updated_at = ?3 WHERE attempt_id = ?1",
        params![receipt.tuple.attempt_id, status.as_str(), now],
    )
    .map_err(schema::map_sqlite)?;
    tx.execute(
        "UPDATE episodes SET state = ?2, updated_at = ?3 WHERE episode_id = ?1",
        params![receipt.tuple.episode_id, status.as_str(), now],
    )
    .map_err(schema::map_sqlite)?;
    project_completion(
        tx,
        current,
        receipt,
        &CompletionRecord::new(
            receipt.tuple.lineage()?,
            status,
            receipt.terminal_ref.clone(),
            receipt.checkpoint_sequence,
            receipt.result_digest.clone(),
        )?,
        now,
    )
}

fn unresolved_count(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    episode_id: &str,
) -> Result<i64, super::types::ExecutionStoreError> {
    tx.query_row(sql, [episode_id], |row| row.get::<_, i64>(0))
        .map_err(schema::map_sqlite)
}

pub(super) fn read_existing_completion(
    tx: &rusqlite::Transaction<'_>,
    episode_id: &str,
) -> Result<Option<CompletionRecord>, super::types::ExecutionStoreError> {
    tx.query_row(
        "SELECT run_id, episode_id, attempt_id, trajectory_id, status, terminal_ref,
         checkpoint_sequence, result_digest FROM completions WHERE episode_id = ?1",
        [episode_id],
        super::store_completion::read_completion,
    )
    .optional()
    .map_err(schema::map_sqlite)
}
