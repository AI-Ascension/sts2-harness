// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::store_worker_queries::read_control;
use super::types::{
    WorkerBoot, WorkerControlMode, WorkerControlRequest, WorkerControlState, WorkerOwnerProof,
};

pub(crate) const CONTROL_SELECT: &str = "SELECT control_id, deployment_id, worker_owner_id,
    worker_profile_digest, worker_boot_id, watchdog_boot_id, mode, mode_sequence,
    generation, authenticated, admitting FROM worker_control WHERE control_id = 1";

impl ExecutionStore {
    /// Starts a fresh worker process generation. Every boot begins stopped and non-admitting;
    /// retained handoffs are deliberately not released by this transition.
    pub fn start_worker_boot(
        &mut self,
        boot: &WorkerBoot,
    ) -> Result<WorkerControlState, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        boot.validate()?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(CONTROL_SELECT, [], read_control)
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(current) = current {
            if current.worker_boot_id == boot.worker_boot_id
                || current.deployment_id != boot.deployment_id
                || current.worker_owner_id != boot.worker_owner_id
                || current.worker_profile_digest != boot.worker_profile_digest
            {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            let known = tx
                .query_row(
                    "SELECT 1 FROM worker_control_boots WHERE boot_id = ?1",
                    [boot.worker_boot_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(schema::map_sqlite)?;
            if known.is_some() {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            let generation = current
                .generation
                .checked_add(1)
                .ok_or(super::types::ExecutionStoreError::InvalidIdentity)?;
            let generation = i64::try_from(generation)
                .map_err(|_| super::types::ExecutionStoreError::InvalidIdentity)?;
            tx.execute(
                "INSERT INTO worker_control_boots(boot_id, kind, generation, created_at)
                 VALUES (?1, 'worker', ?2, ?3)",
                params![boot.worker_boot_id, generation, now],
            )
            .map_err(schema::map_sqlite)?;
            tx.execute(
                "UPDATE worker_control SET deployment_id = ?2, worker_owner_id = ?3,
                 worker_profile_digest = ?4, worker_boot_id = ?5,
                 mode = 'stopped', generation = ?6,
                 authenticated = 0, admitting = 0, updated_at = ?7 WHERE control_id = 1",
                params![
                    1_i64,
                    boot.deployment_id,
                    boot.worker_owner_id,
                    boot.worker_profile_digest,
                    boot.worker_boot_id,
                    generation,
                    now
                ],
            )
            .map_err(schema::map_sqlite)?;
        } else {
            let known = tx
                .query_row(
                    "SELECT 1 FROM worker_control_boots WHERE boot_id = ?1",
                    [boot.worker_boot_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(schema::map_sqlite)?;
            if known.is_some() {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.execute(
                "INSERT INTO worker_control(control_id, deployment_id, worker_owner_id,
                 worker_profile_digest, worker_boot_id, watchdog_boot_id, mode, mode_sequence,
                 generation, authenticated, admitting, updated_at)
                 VALUES (1, ?1, ?2, ?3, ?4, NULL, 'stopped', 0, 1, 0, 0, ?5)",
                params![
                    boot.deployment_id,
                    boot.worker_owner_id,
                    boot.worker_profile_digest,
                    boot.worker_boot_id,
                    now
                ],
            )
            .map_err(schema::map_sqlite)?;
            tx.execute(
                "INSERT INTO worker_control_boots(boot_id, kind, generation, created_at)
                 VALUES (?1, 'worker', 1, ?2)",
                params![boot.worker_boot_id, now],
            )
            .map_err(schema::map_sqlite)?;
        }
        append_event(
            &tx,
            "worker_control",
            &boot.worker_boot_id,
            WorkerControlMode::Stopped.as_str(),
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_control()?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }

    pub fn worker_control(
        &self,
    ) -> Result<Option<WorkerControlState>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
            .query_row(CONTROL_SELECT, [], read_control)
            .optional()
            .map_err(schema::map_sqlite)
    }

    /// Applies an authenticated owner control update. The owner proof is supplied by the future
    /// protected transport; an increasing sequence alone never authorizes a new watchdog boot.
    pub fn set_worker_control_mode(
        &mut self,
        request: &WorkerControlRequest,
        owner_proof: &WorkerOwnerProof,
    ) -> Result<WorkerControlState, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        request.validate()?;
        if !owner_proof.is_present() {
            return Err(super::types::ExecutionStoreError::InvalidIdentity);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current =
            tx.query_row(CONTROL_SELECT, [], read_control)
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        super::types::ExecutionStoreError::Missing
                    }
                    other => schema::map_sqlite(other),
                })?;
        if current.deployment_id != request.deployment_id
            || current.worker_owner_id != request.worker_owner_id
            || current.worker_profile_digest != request.worker_profile_digest
            || current.worker_boot_id != request.worker_boot_id
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let same_watchdog = current
            .watchdog_boot_id
            .as_deref()
            .is_some_and(|boot| boot == request.watchdog_boot_id);
        if same_watchdog {
            if request.mode_sequence == current.mode_sequence
                && request.mode == current.mode
                && current.authenticated
            {
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(current);
            }
            if request.mode_sequence <= current.mode_sequence {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
        } else {
            let known = tx
                .query_row(
                    "SELECT 1 FROM worker_control_boots WHERE boot_id = ?1",
                    [request.watchdog_boot_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(schema::map_sqlite)?;
            if known.is_some() {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.execute(
                "INSERT INTO worker_control_boots(boot_id, kind, generation, created_at)
                 VALUES (?1, 'watchdog', ?2, ?3)",
                params![request.watchdog_boot_id, current.generation, now],
            )
            .map_err(schema::map_sqlite)?;
        }
        let generation = if same_watchdog {
            current.generation
        } else {
            current
                .generation
                .checked_add(1)
                .ok_or(super::types::ExecutionStoreError::InvalidIdentity)?
        };
        let generation = i64::try_from(generation)
            .map_err(|_| super::types::ExecutionStoreError::InvalidIdentity)?;
        tx.execute(
            "UPDATE worker_control SET watchdog_boot_id = ?2, mode = ?3,
             mode_sequence = ?4, generation = ?5, authenticated = 1,
             admitting = ?6, updated_at = ?7 WHERE control_id = 1",
            params![
                1_i64,
                request.watchdog_boot_id,
                request.mode.as_str(),
                i64::try_from(request.mode_sequence)
                    .map_err(|_| super::types::ExecutionStoreError::InvalidIdentity)?,
                generation,
                i64::from(request.mode.admits()),
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "worker_control",
            &request.worker_boot_id,
            request.mode.as_str(),
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.worker_control()?
            .ok_or(super::types::ExecutionStoreError::Corrupt)
    }
}
