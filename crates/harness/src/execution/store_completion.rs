// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::types::{CompletionRecord, CompletionStatus, ExecutionLineage};

impl ExecutionStore {
    pub fn record_completion(
        &mut self,
        completion: &CompletionRecord,
    ) -> Result<bool, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        completion.validate()?;
        let sequence = i64::try_from(completion.checkpoint_sequence)
            .map_err(|_| super::types::ExecutionStoreError::InvalidCompletion)?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT run_id, current_attempt_id, current_trajectory_id, state
                 FROM episodes WHERE episode_id = ?1",
                [completion.lineage.episode_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if current.0 != completion.lineage.run_id
            || current.1 != completion.lineage.attempt_id
            || current.2 != completion.lineage.trajectory_id
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let unresolved_operations: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM operations WHERE episode_id = ?1 AND state IN
                 ('intent_recorded', 'may_have_been_dispatched', 'accepted', 'unknown')",
                [completion.lineage.episode_id.as_str()],
                |row| row.get(0),
            )
            .map_err(schema::map_sqlite)?;
        let unresolved_decisions: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE episode_id = ?1 AND state IN ('pending', 'unknown')",
                [completion.lineage.episode_id.as_str()],
                |row| row.get(0),
            )
            .map_err(schema::map_sqlite)?;
        let unresolved_reservations: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM provider_reservations WHERE episode_id = ?1
                 AND state IN ('reserved', 'unknown')",
                [completion.lineage.episode_id.as_str()],
                |row| row.get(0),
            )
            .map_err(schema::map_sqlite)?;
        if unresolved_operations != 0 || unresolved_decisions != 0 || unresolved_reservations != 0 {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let existing = tx
            .query_row(
                "SELECT run_id, episode_id, attempt_id, trajectory_id, status, terminal_ref,
                 checkpoint_sequence, result_digest FROM completions WHERE episode_id = ?1",
                [completion.lineage.episode_id.as_str()],
                read_completion,
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(existing) = existing {
            if existing != *completion {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(false);
        }
        let checkpoint_exists = tx
            .query_row(
                "SELECT COUNT(*) FROM checkpoints WHERE episode_id = ?1 AND attempt_id = ?2
                 AND sequence = ?3",
                params![
                    completion.lineage.episode_id,
                    completion.lineage.attempt_id,
                    sequence
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(schema::map_sqlite)?;
        if checkpoint_exists != 1 {
            return Err(super::types::ExecutionStoreError::InvalidCompletion);
        }
        if current.3 != "active" {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "INSERT INTO completions(episode_id, run_id, attempt_id, trajectory_id, status,
             terminal_ref, checkpoint_sequence, result_digest, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                completion.lineage.episode_id,
                completion.lineage.run_id,
                completion.lineage.attempt_id,
                completion.lineage.trajectory_id,
                completion.status.as_str(),
                completion.terminal_ref,
                sequence,
                completion.result_digest,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "UPDATE attempts SET state = ?2, updated_at = ?3 WHERE attempt_id = ?1",
            params![
                completion.lineage.attempt_id,
                completion.status.as_str(),
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "UPDATE episodes SET state = ?2, updated_at = ?3 WHERE episode_id = ?1",
            params![
                completion.lineage.episode_id,
                completion.status.as_str(),
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "episode",
            &completion.lineage.episode_id,
            completion.status.as_str(),
            Some(&completion.result_digest),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(true)
    }
}

pub(crate) fn read_completion(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompletionRecord> {
    let lineage = ExecutionLineage::new(
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let status = CompletionStatus::from_str(&row.get::<_, String>(4)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    CompletionRecord::new(
        lineage,
        status,
        row.get::<_, String>(5)?,
        u64::try_from(row.get::<_, i64>(6)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        row.get::<_, String>(7)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)
}
