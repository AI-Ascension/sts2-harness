// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::types::{RecoveryDisposition, StoredAttempt, valid_reference};

impl ExecutionStore {
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
                params![&attempt_id, &state, now],
            )
            .map_err(schema::map_sqlite)?;
            tx.execute(
                "UPDATE episodes SET state = ?2, updated_at = ?3 WHERE episode_id = ?1",
                params![&episode_id, &state, now],
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
}
