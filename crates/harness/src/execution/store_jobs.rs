// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::types::{JobClaim, JobClaimOutcome, JobState, StoredJob, valid_id, valid_reference};

const MAX_JOBS: i64 = 4_096;

impl ExecutionStore {
    pub fn admit_job(
        &mut self,
        job_id: &str,
        episode_id: &str,
        payload_digest: &str,
    ) -> Result<StoredJob, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_id(job_id) || !valid_id(episode_id) || !valid_reference(payload_digest) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let existing = tx
            .query_row(
                "SELECT job_id, episode_id, payload_digest, state, claim_token, worker_id, result_ref
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                read_job,
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(existing) = existing {
            if existing.episode_id != episode_id || existing.payload_digest != payload_digest {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(existing);
        }
        let count = tx
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))
            .map_err(schema::map_sqlite)?;
        if count >= MAX_JOBS {
            return Err(super::types::ExecutionStoreError::Capacity);
        }
        tx.execute(
            "INSERT INTO jobs(job_id, episode_id, payload_digest, state, claim_token, worker_id,
             result_ref, created_at, updated_at) VALUES (?1, ?2, ?3, 'admitted', NULL, NULL, NULL, ?4, ?4)",
            params![job_id, episode_id, payload_digest, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(&tx, "job", job_id, JobState::Admitted.as_str(), None, now)?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.job(job_id)
    }

    /// Claims a job in one transaction. A completed job is returned as terminal and must not be
    /// rerun after a worker crash or an acknowledgement loss.
    pub fn claim_job(
        &mut self,
        job_id: &str,
        worker_id: &str,
    ) -> Result<JobClaimOutcome, super::types::ExecutionStoreError> {
        let token = format!("claim-{job_id}-{worker_id}");
        self.claim_job_with_token(job_id, worker_id, &token)
    }

    pub fn claim_job_with_token(
        &mut self,
        job_id: &str,
        worker_id: &str,
        claim_token: &str,
    ) -> Result<JobClaimOutcome, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_id(job_id) || !valid_id(worker_id) || !valid_id(claim_token) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT job_id, episode_id, payload_digest, state, claim_token, worker_id, result_ref
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                read_job,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if current.state == JobState::Completed {
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(JobClaimOutcome::AlreadyCompleted(current));
        }
        if current.state == JobState::Failed {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.state == JobState::Claimed {
            let claim = current
                .claim
                .ok_or(super::types::ExecutionStoreError::Corrupt)?;
            if claim.worker_id == worker_id && claim.claim_token == claim_token {
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(JobClaimOutcome::AlreadyClaimed(claim));
            }
            return Err(super::types::ExecutionStoreError::Busy);
        }
        tx.execute(
            "UPDATE jobs SET state = 'claimed', claim_token = ?2, worker_id = ?3, updated_at = ?4
             WHERE job_id = ?1 AND state = 'admitted'",
            params![job_id, claim_token, worker_id, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(&tx, "job", job_id, JobState::Claimed.as_str(), None, now)?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(JobClaimOutcome::Claimed(JobClaim {
            job_id: job_id.to_owned(),
            episode_id: current.episode_id,
            claim_token: claim_token.to_owned(),
            worker_id: worker_id.to_owned(),
        }))
    }

    /// Completion is committed before the caller is allowed to acknowledge or exit. The claim
    /// token fences a late worker from completing a job it no longer owns.
    pub fn complete_job(
        &mut self,
        job_id: &str,
        claim_token: &str,
        result_ref: &str,
    ) -> Result<StoredJob, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_id(job_id) || !valid_id(claim_token) || !valid_reference(result_ref) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT job_id, episode_id, payload_digest, state, claim_token, worker_id, result_ref
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                read_job,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if current.state == JobState::Completed {
            if current.result_ref.as_deref() == Some(result_ref) {
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(current);
            }
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.state != JobState::Claimed
            || current
                .claim
                .as_ref()
                .is_none_or(|claim| claim.claim_token != claim_token)
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "UPDATE jobs SET state = 'completed', result_ref = ?2, updated_at = ?3
             WHERE job_id = ?1 AND state = 'claimed' AND claim_token = ?4",
            params![job_id, result_ref, now, claim_token],
        )
        .map_err(schema::map_sqlite)?;
        append_event(&tx, "job", job_id, JobState::Completed.as_str(), None, now)?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.job(job_id)
    }

    pub fn acknowledge_job(
        &self,
        job_id: &str,
    ) -> Result<StoredJob, super::types::ExecutionStoreError> {
        let job = self.job(job_id)?;
        if job.state != JobState::Completed {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        Ok(job)
    }

    /// Permanently records a failed job. A failed job is not implicitly claimable on restart;
    /// retry admission must create a new job identity or an explicit policy-level retry record.
    pub fn fail_job(
        &mut self,
        job_id: &str,
        claim_token: &str,
        result_ref: &str,
    ) -> Result<StoredJob, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_id(job_id) || !valid_id(claim_token) || !valid_reference(result_ref) {
            return Err(super::types::ExecutionStoreError::InvalidJob);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT job_id, episode_id, payload_digest, state, claim_token, worker_id, result_ref
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                read_job,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if current.state == JobState::Failed {
            if current.result_ref.as_deref() == Some(result_ref) {
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(current);
            }
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if current.state != JobState::Claimed
            || current
                .claim
                .as_ref()
                .is_none_or(|claim| claim.claim_token != claim_token)
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "UPDATE jobs SET state = 'failed', result_ref = ?2, updated_at = ?3
             WHERE job_id = ?1 AND state = 'claimed' AND claim_token = ?4",
            params![job_id, result_ref, now, claim_token],
        )
        .map_err(schema::map_sqlite)?;
        append_event(&tx, "job", job_id, JobState::Failed.as_str(), None, now)?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.job(job_id)
    }

    pub fn job(&self, job_id: &str) -> Result<StoredJob, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
            .query_row(
                "SELECT job_id, episode_id, payload_digest, state, claim_token, worker_id, result_ref
                 FROM jobs WHERE job_id = ?1",
                [job_id],
                read_job,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })
    }
}

fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredJob> {
    let state =
        JobState::from_str(&row.get::<_, String>(3)?).ok_or(rusqlite::Error::InvalidQuery)?;
    let claim = match (
        row.get::<_, Option<String>>(4)?,
        row.get::<_, Option<String>>(5)?,
    ) {
        (Some(claim_token), Some(worker_id)) => Some(JobClaim {
            job_id: row.get(0)?,
            episode_id: row.get(1)?,
            claim_token,
            worker_id,
        }),
        (None, None) => None,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(StoredJob {
        job_id: row.get(0)?,
        episode_id: row.get(1)?,
        payload_digest: row.get(2)?,
        state,
        claim,
        result_ref: row.get(6)?,
    })
}
