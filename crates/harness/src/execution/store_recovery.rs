// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::schema;
use super::store_completion::read_completion;
use super::store_core::ExecutionStore;
use super::types::{
    AttemptKind, AttemptState, Checkpoint, CompletionRecord, ExecutionFingerprint,
    ExecutionLineage, ResumeState, StoredAttempt, StoredDecision, StoredEpisode,
};

impl ExecutionStore {
    /// Explicitly opens an episode for continuation or creates it only when no durable record
    /// exists. Existing state is never routed through the ordinary new-episode path.
    pub fn resume_or_start_episode(
        &mut self,
        lineage: &ExecutionLineage,
        approved_fingerprint: &ExecutionFingerprint,
    ) -> Result<ResumeState, super::types::ExecutionStoreError> {
        match self.resume_episode(&lineage.episode_id, approved_fingerprint)? {
            ResumeState::New => {
                self.start_episode(lineage, approved_fingerprint)?;
                self.resume_episode(&lineage.episode_id, approved_fingerprint)
            }
            state => {
                if self.load_episode(&lineage.episode_id)?.lineage != *lineage {
                    return Err(super::types::ExecutionStoreError::Conflict);
                }
                Ok(state)
            }
        }
    }

    /// Admission gate for a new model decision. Callers must reconcile every returned pending
    /// operation/decision and obtain a fresh legal observation before invoking this method.
    pub fn resume_for_decision(
        &self,
        episode_id: &str,
        approved_fingerprint: &ExecutionFingerprint,
    ) -> Result<Option<Checkpoint>, super::types::ExecutionStoreError> {
        match self.resume_episode(episode_id, approved_fingerprint)? {
            ResumeState::Ready {
                checkpoint,
                pending_operations,
                pending_decisions,
            } if pending_operations.is_empty()
                && pending_decisions.is_empty()
                && self.pending_provider_reservations(episode_id)?.is_empty() =>
            {
                Ok(checkpoint.map(|checkpoint| *checkpoint))
            }
            ResumeState::Ready { .. } => Err(super::types::ExecutionStoreError::Conflict),
            ResumeState::New => Err(super::types::ExecutionStoreError::Missing),
            ResumeState::Completed(_) => Err(super::types::ExecutionStoreError::Conflict),
            ResumeState::ReconstructionRequired { .. } | ResumeState::InterruptedUnknown { .. } => {
                Err(super::types::ExecutionStoreError::Incompatible)
            }
        }
    }

    pub fn load_episode(
        &self,
        episode_id: &str,
    ) -> Result<StoredEpisode, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        let row = self
            .connection
            .query_row(
                "SELECT run_id, current_attempt_id, current_trajectory_id, state
                 FROM episodes WHERE episode_id = ?1",
                [episode_id],
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
        let lineage =
            ExecutionLineage::new(row.0, episode_id.to_owned(), row.1.clone(), row.2.clone())
                .map_err(|_| super::types::ExecutionStoreError::Corrupt)?;
        let attempt = self.attempt(&lineage.attempt_id)?;
        let fingerprint = attempt_fingerprint_connection(&self.connection, &lineage.attempt_id)?;
        let state = episode_state(&row.3).ok_or(super::types::ExecutionStoreError::Corrupt)?;
        let last_checkpoint = self.last_checkpoint(episode_id)?;
        let completion = self.completion(episode_id)?;
        if attempt.lineage != lineage || attempt.state != state {
            return Err(super::types::ExecutionStoreError::Corrupt);
        }
        Ok(StoredEpisode {
            lineage,
            fingerprint,
            state,
            last_checkpoint,
            completion,
        })
    }

    pub fn attempt(
        &self,
        attempt_id: &str,
    ) -> Result<StoredAttempt, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
            .query_row(
                "SELECT run_id, episode_id, attempt_id, trajectory_id, parent_attempt_id,
                 kind, state FROM attempts WHERE attempt_id = ?1",
                [attempt_id],
                |row| {
                    let lineage = ExecutionLineage::new(
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    )
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;
                    let kind = AttemptKind::from_str(&row.get::<_, String>(5)?)
                        .ok_or(rusqlite::Error::InvalidQuery)?;
                    let state = AttemptState::from_str(&row.get::<_, String>(6)?)
                        .ok_or(rusqlite::Error::InvalidQuery)?;
                    Ok(StoredAttempt {
                        lineage,
                        kind,
                        state,
                        parent_attempt_id: row.get(4)?,
                    })
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                rusqlite::Error::InvalidQuery => super::types::ExecutionStoreError::Corrupt,
                other => schema::map_sqlite(other),
            })
    }

    /// Returns every attempt in creation order, including failed and superseded attempts. The
    /// history is intentionally never collapsed into the current attempt during reconstruction.
    pub fn attempts_for_episode(
        &self,
        episode_id: &str,
    ) -> Result<Vec<StoredAttempt>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id, episode_id, attempt_id, trajectory_id, parent_attempt_id,
                 kind, state FROM attempts WHERE episode_id = ?1
                 ORDER BY created_at, attempt_id",
            )
            .map_err(schema::map_sqlite)?;
        let rows = statement
            .query_map([episode_id], read_attempt)
            .map_err(schema::map_sqlite)?;
        rows.map(|row| row.map_err(schema::map_sqlite))
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn completion(
        &self,
        episode_id: &str,
    ) -> Result<Option<CompletionRecord>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
            .query_row(
                "SELECT run_id, episode_id, attempt_id, trajectory_id, status, terminal_ref,
                 checkpoint_sequence, result_digest FROM completions WHERE episode_id = ?1",
                [episode_id],
                read_completion,
            )
            .optional()
            .map_err(|error| match error {
                rusqlite::Error::InvalidQuery => super::types::ExecutionStoreError::Corrupt,
                other => schema::map_sqlite(other),
            })
    }

    /// Loads durable state for a caller that will reconcile every pending operation before making
    /// another decision. A completed episode is returned as terminal and is never rerun.
    pub fn resume_episode(
        &self,
        episode_id: &str,
        approved_fingerprint: &ExecutionFingerprint,
    ) -> Result<ResumeState, super::types::ExecutionStoreError> {
        approved_fingerprint.validate()?;
        let episode = match self.load_episode(episode_id) {
            Ok(episode) => episode,
            Err(super::types::ExecutionStoreError::Missing) => return Ok(ResumeState::New),
            Err(error) => return Err(error),
        };
        if episode.fingerprint != *approved_fingerprint {
            return Ok(ResumeState::ReconstructionRequired {
                reason: String::from(
                    "approved seed/build/state/config/provider fingerprint changed",
                ),
            });
        }
        if let Some(completion) = episode.completion.clone() {
            return Ok(ResumeState::Completed(completion));
        }
        if matches!(
            episode.state,
            AttemptState::InterruptedUnknown | AttemptState::Quarantined
        ) {
            return Ok(ResumeState::InterruptedUnknown {
                reason: String::from("current attempt is quarantined or interrupted-unknown"),
            });
        }
        if episode.state == AttemptState::Failed {
            return Ok(ResumeState::ReconstructionRequired {
                reason: String::from("current attempt is failed and cannot continue in place"),
            });
        }
        let pending_operations = self.pending_operations(episode_id)?;
        let pending_decisions = self.pending_decisions(episode_id)?;
        Ok(ResumeState::Ready {
            checkpoint: episode.last_checkpoint.map(Box::new),
            pending_operations,
            pending_decisions,
        })
    }

    pub fn pending_decisions(
        &self,
        episode_id: &str,
    ) -> Result<Vec<StoredDecision>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        let query = super::store_provider::decision_query(
            "WHERE episode_id = ?1 AND state IN ('pending', 'unknown')
             ORDER BY created_at, execution_id",
        );
        let mut statement = self
            .connection
            .prepare(&query)
            .map_err(schema::map_sqlite)?;
        let rows = statement
            .query_map([episode_id], super::store_provider::read_decision)
            .map_err(schema::map_sqlite)?;
        rows.map(|row| row.map_err(schema::map_sqlite))
            .collect::<Result<Vec<_>, _>>()
    }
}

fn episode_state(value: &str) -> Option<AttemptState> {
    match value {
        "active" => Some(AttemptState::Active),
        "completed" => Some(AttemptState::Completed),
        "failed" => Some(AttemptState::Failed),
        "interrupted_unknown" => Some(AttemptState::InterruptedUnknown),
        "quarantined" => Some(AttemptState::Quarantined),
        _ => None,
    }
}

fn read_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAttempt> {
    let lineage = ExecutionLineage::new(
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let kind =
        AttemptKind::from_str(&row.get::<_, String>(5)?).ok_or(rusqlite::Error::InvalidQuery)?;
    let state =
        AttemptState::from_str(&row.get::<_, String>(6)?).ok_or(rusqlite::Error::InvalidQuery)?;
    Ok(StoredAttempt {
        lineage,
        kind,
        state,
        parent_attempt_id: row.get(4)?,
    })
}

fn attempt_fingerprint_connection(
    connection: &rusqlite::Connection,
    attempt_id: &str,
) -> Result<ExecutionFingerprint, super::types::ExecutionStoreError> {
    connection
        .query_row(
            "SELECT seed, build_digest, state_digest, config_digest, provider_digest
             FROM attempts WHERE attempt_id = ?1",
            [attempt_id],
            |row| {
                ExecutionFingerprint::new(
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                )
                .map_err(|_| rusqlite::Error::InvalidQuery)
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
            other => schema::map_sqlite(other),
        })
}
