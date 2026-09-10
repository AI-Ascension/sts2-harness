// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{
    ExecutionStore, append_event, attempt_fingerprint, ensure_current_lineage,
};
use super::types::{CatalogEvidence, Checkpoint, ExecutionLineage, MAX_CATALOG_BYTES};

impl ExecutionStore {
    /// Stores a verified public boundary. A sequence can be retried only with byte-identical
    /// content; a changed checkpoint is rejected rather than silently replacing replay evidence.
    pub fn save_checkpoint(
        &mut self,
        checkpoint: &Checkpoint,
    ) -> Result<bool, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        checkpoint.lineage.validate()?;
        checkpoint.fingerprint.validate()?;
        checkpoint.validate(self.config.max_payload_bytes)?;
        let sequence = i64::try_from(checkpoint.sequence)
            .map_err(|_| super::types::ExecutionStoreError::InvalidCheckpoint)?;
        let generation = i64::try_from(checkpoint.generation)
            .map_err(|_| super::types::ExecutionStoreError::InvalidCheckpoint)?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT current_attempt_id, current_trajectory_id FROM episodes
                 WHERE episode_id = ?1",
                [checkpoint.lineage.episode_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(schema::map_sqlite)?
            .ok_or(super::types::ExecutionStoreError::Missing)?;
        if current.0 != checkpoint.lineage.attempt_id
            || current.1 != checkpoint.lineage.trajectory_id
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        ensure_current_lineage(&tx, &checkpoint.lineage)?;
        if attempt_fingerprint(&tx, &checkpoint.lineage.attempt_id)? != checkpoint.fingerprint {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let existing = tx
            .query_row(
                &format!(
                    "SELECT state_id, generation, seed, build_digest, state_digest,
                     config_digest, provider_digest, observation, legal_actions_digest,
                     CASE WHEN legal_actions_raw IS NULL THEN NULL
                          WHEN typeof(legal_actions_raw) = 'blob'
                          THEN substr(legal_actions_raw, 1, {}) ELSE NULL END,
                     CASE WHEN legal_actions_raw IS NULL THEN 0
                          WHEN typeof(legal_actions_raw) <> 'blob' THEN 1
                          WHEN length(legal_actions_raw) > {} THEN 2
                          ELSE 0 END
                     FROM checkpoints WHERE episode_id = ?1 AND attempt_id = ?2 AND sequence = ?3",
                    MAX_CATALOG_BYTES + 1,
                    MAX_CATALOG_BYTES
                ),
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
                        row.get::<_, Option<Vec<u8>>>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(existing) = existing {
            if existing.10 != 0 {
                return Err(super::types::ExecutionStoreError::Corrupt);
            }
            let same = existing.0 == checkpoint.state_id
                && existing.1 == generation
                && existing.2 == checkpoint.fingerprint.seed
                && existing.3 == checkpoint.fingerprint.build_digest
                && existing.4 == checkpoint.fingerprint.state_digest
                && existing.5 == checkpoint.fingerprint.config_digest
                && existing.6 == checkpoint.fingerprint.provider_digest
                && existing.7 == checkpoint.observation
                && existing.8 == checkpoint.legal_actions_digest
                && existing.9 == checkpoint.catalog_raw;
            if !same {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO checkpoints(episode_id, attempt_id, sequence, state_id, generation,
             seed, build_digest, state_digest, config_digest, provider_digest, observation,
             legal_actions_digest, legal_actions_raw, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                checkpoint.lineage.episode_id,
                checkpoint.lineage.attempt_id,
                sequence,
                checkpoint.state_id,
                generation,
                checkpoint.fingerprint.seed,
                checkpoint.fingerprint.build_digest,
                checkpoint.fingerprint.state_digest,
                checkpoint.fingerprint.config_digest,
                checkpoint.fingerprint.provider_digest,
                checkpoint.observation,
                checkpoint.legal_actions_digest,
                checkpoint.catalog_raw,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "UPDATE episodes SET updated_at = ?2 WHERE episode_id = ?1",
            params![checkpoint.lineage.episode_id, now],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "UPDATE attempts SET updated_at = ?2 WHERE attempt_id = ?1",
            params![checkpoint.lineage.attempt_id, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "checkpoint",
            &format!("{}:{}", checkpoint.lineage.episode_id, checkpoint.sequence),
            "verified",
            Some(&checkpoint.fingerprint.state_digest),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(true)
    }

    pub fn last_checkpoint(
        &self,
        episode_id: &str,
    ) -> Result<Option<Checkpoint>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        let current = self
            .connection
            .query_row(
                "SELECT run_id, current_attempt_id, current_trajectory_id FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        let Some((run_id, attempt_id, trajectory_id)) = current else {
            return Err(super::types::ExecutionStoreError::Missing);
        };
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT sequence, state_id, generation, seed, build_digest, state_digest,
                 config_digest, provider_digest, observation, legal_actions_digest,
                 CASE WHEN legal_actions_raw IS NULL THEN NULL
                      WHEN typeof(legal_actions_raw) = 'blob'
                      THEN substr(legal_actions_raw, 1, {}) ELSE NULL END,
                 CASE WHEN legal_actions_raw IS NULL THEN 0
                      WHEN typeof(legal_actions_raw) <> 'blob' THEN 1
                      WHEN length(legal_actions_raw) > {} THEN 2
                      ELSE 0 END
                 FROM checkpoints WHERE episode_id = ?1 AND attempt_id = ?2
                 ORDER BY sequence DESC LIMIT 1",
                MAX_CATALOG_BYTES + 1,
                MAX_CATALOG_BYTES
            ))
            .map_err(schema::map_sqlite)?;
        statement
            .query_row(params![episode_id, attempt_id], |row| {
                let lineage = ExecutionLineage::new(
                    run_id.clone(),
                    episode_id.to_owned(),
                    attempt_id.clone(),
                    trajectory_id.clone(),
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
                let fingerprint = super::types::ExecutionFingerprint::new(
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
                if row.get::<_, i64>(11)? != 0 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Checkpoint::new_with_optional_catalog(
                    lineage,
                    u64::try_from(row.get::<_, i64>(0)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    row.get::<_, String>(1)?,
                    u64::try_from(row.get::<_, i64>(2)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    fingerprint,
                    row.get::<_, Vec<u8>>(8)?,
                    CatalogEvidence::new(
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<Vec<u8>>>(10)?,
                    ),
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .optional()
            .map_err(schema::map_sqlite)
    }
}
