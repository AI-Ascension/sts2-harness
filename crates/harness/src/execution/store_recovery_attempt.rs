// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{
    ExecutionStore, append_event, attempt_fingerprint, ensure_current_lineage, insert_attempt,
};
use super::types::{
    AttemptKind, Checkpoint, ExecutionFingerprint, ExecutionLineage, RecoveryDisposition,
    StoredAttempt, valid_reference,
};

impl ExecutionStore {
    pub fn mark_interrupted_unknown(
        &mut self,
        episode_id: &str,
        reason: &str,
    ) -> Result<StoredAttempt, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_reference(reason) {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let (attempt_id, episode_state) = tx
            .query_row(
                "SELECT current_attempt_id, state FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if episode_state != "active" {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "UPDATE attempts SET state = 'interrupted_unknown', updated_at = ?2
             WHERE attempt_id = ?1",
            params![attempt_id, now],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "UPDATE episodes SET state = 'interrupted_unknown', updated_at = ?2
             WHERE episode_id = ?1",
            params![episode_id, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "attempt",
            &attempt_id,
            "interrupted_unknown",
            Some(reason),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.attempt(&attempt_id)
    }

    pub fn record_recovery_disposition(
        &mut self,
        episode_id: &str,
        disposition: RecoveryDisposition,
        reason: &str,
    ) -> Result<StoredAttempt, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_reference(episode_id) || !valid_reference(reason) {
            return Err(super::types::ExecutionStoreError::InvalidOperation);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let (attempt_id, episode_state) = tx
            .query_row(
                "SELECT current_attempt_id, state FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if disposition == RecoveryDisposition::InterruptedUnknown && episode_state != "active" {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let state = if disposition == RecoveryDisposition::InterruptedUnknown {
            "interrupted_unknown"
        } else {
            "active"
        };
        if disposition == RecoveryDisposition::InterruptedUnknown {
            tx.execute(
                "UPDATE attempts SET state = ?2, updated_at = ?3 WHERE attempt_id = ?1",
                params![attempt_id, state, now],
            )
            .map_err(schema::map_sqlite)?;
            tx.execute(
                "UPDATE episodes SET state = ?2, updated_at = ?3 WHERE episode_id = ?1",
                params![episode_id, state, now],
            )
            .map_err(schema::map_sqlite)?;
        }
        append_event(
            &tx,
            "episode",
            episode_id,
            disposition.as_str(),
            Some(reason),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.attempt(&attempt_id)
    }

    /// Creates a new attempt only from an exact approved checkpoint. It never clears the old
    /// attempt or its operations, and therefore cannot be used to evade an unknown mutation.
    pub fn reconstruct_attempt(
        &mut self,
        checkpoint: &Checkpoint,
        new_lineage: &ExecutionLineage,
        approved_fingerprint: &ExecutionFingerprint,
        provenance_ref: &str,
    ) -> Result<StoredAttempt, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if checkpoint.fingerprint != *approved_fingerprint
            || checkpoint.lineage.run_id != new_lineage.run_id
            || checkpoint.lineage.episode_id != new_lineage.episode_id
            || !valid_reference(provenance_ref)
        {
            return Err(super::types::ExecutionStoreError::Incompatible);
        }
        checkpoint.lineage.validate()?;
        new_lineage.validate()?;
        approved_fingerprint.validate()?;
        checkpoint.validate(self.config.max_payload_bytes)?;
        let sequence = i64::try_from(checkpoint.sequence)
            .map_err(|_| super::types::ExecutionStoreError::InvalidCheckpoint)?;
        if !self
            .pending_operations(&checkpoint.lineage.episode_id)?
            .is_empty()
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if !self
            .pending_decisions(&checkpoint.lineage.episode_id)?
            .is_empty()
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if !self
            .pending_provider_reservations(&checkpoint.lineage.episode_id)?
            .is_empty()
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT current_attempt_id FROM episodes WHERE episode_id = ?1",
                [new_lineage.episode_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if current != checkpoint.lineage.attempt_id {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let old_fingerprint = attempt_fingerprint(&tx, &current)?;
        if old_fingerprint != *approved_fingerprint {
            return Err(super::types::ExecutionStoreError::Incompatible);
        }
        ensure_current_lineage(&tx, &checkpoint.lineage)?;
        let stored_checkpoint = tx
            .query_row(
                "SELECT state_id, generation, seed, build_digest, state_digest,
                 config_digest, provider_digest, observation, legal_actions_digest
                 FROM checkpoints WHERE episode_id = ?1 AND attempt_id = ?2 AND sequence = ?3",
                params![
                    checkpoint.lineage.episode_id,
                    checkpoint.lineage.attempt_id,
                    sequence
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(schema::map_sqlite)?
            .ok_or(super::types::ExecutionStoreError::InvalidCheckpoint)?;
        let stored_generation = u64::try_from(stored_checkpoint.1)
            .map_err(|_| super::types::ExecutionStoreError::InvalidCheckpoint)?;
        if stored_checkpoint.0 != checkpoint.state_id
            || stored_generation != checkpoint.generation
            || stored_checkpoint.2 != checkpoint.fingerprint.seed
            || stored_checkpoint.3 != checkpoint.fingerprint.build_digest
            || stored_checkpoint.4 != checkpoint.fingerprint.state_digest
            || stored_checkpoint.5 != checkpoint.fingerprint.config_digest
            || stored_checkpoint.6 != checkpoint.fingerprint.provider_digest
            || stored_checkpoint.7 != checkpoint.observation
            || stored_checkpoint.8 != checkpoint.legal_actions_digest
        {
            return Err(super::types::ExecutionStoreError::InvalidCheckpoint);
        }
        tx.execute(
            "UPDATE attempts SET state = 'failed', updated_at = ?2 WHERE attempt_id = ?1",
            params![current, now],
        )
        .map_err(schema::map_sqlite)?;
        insert_attempt(
            &tx,
            new_lineage,
            approved_fingerprint,
            AttemptKind::Reconstruction,
            Some(&current),
            now,
        )?;
        tx.execute(
            "INSERT INTO trajectories(trajectory_id, run_id, episode_id, attempt_id,
             provenance_ref, provenance_digest, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_lineage.trajectory_id,
                new_lineage.run_id,
                new_lineage.episode_id,
                new_lineage.attempt_id,
                provenance_ref,
                checkpoint.fingerprint.state_digest,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        let copied = tx
            .execute(
                "INSERT INTO checkpoints(episode_id, attempt_id, sequence, state_id, generation,
             seed, build_digest, state_digest, config_digest, provider_digest, observation,
             legal_actions_digest, created_at)
             SELECT episode_id, ?2, sequence, state_id, generation, seed, build_digest,
                    state_digest, config_digest, provider_digest, observation,
                    legal_actions_digest, ?3
             FROM checkpoints WHERE episode_id = ?1 AND attempt_id = ?4 AND sequence = ?5",
                params![
                    checkpoint.lineage.episode_id,
                    new_lineage.attempt_id,
                    now,
                    checkpoint.lineage.attempt_id,
                    sequence
                ],
            )
            .map_err(schema::map_sqlite)?;
        if copied != 1 {
            return Err(super::types::ExecutionStoreError::InvalidCheckpoint);
        }
        tx.execute(
            "UPDATE episodes SET current_attempt_id = ?2, current_trajectory_id = ?3,
             state = 'active', updated_at = ?4 WHERE episode_id = ?1",
            params![
                new_lineage.episode_id,
                new_lineage.attempt_id,
                new_lineage.trajectory_id,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "attempt",
            &new_lineage.attempt_id,
            "reconstruction",
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.attempt(&new_lineage.attempt_id)
    }
}
