// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::{ExecutionStore, append_event, ensure_current_lineage};
pub(crate) use super::store_provider_queries::{
    read_decision, read_reservation, same_decision, same_reservation,
};
use super::types::{
    DecisionReference, ProviderReservation, ProviderReservationState, StoredDecision,
};

const MAX_DECISIONS: i64 = 4_096;

impl ExecutionStore {
    /// Records the complete decision input fingerprint before provider I/O. Result bytes are kept
    /// outside this store; only an approved protected reference and digest may be persisted.
    pub fn record_decision(
        &mut self,
        reference: &DecisionReference,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        reference.lineage.validate()?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let existing = tx
            .query_row(
                "SELECT execution_id, run_id, episode_id, attempt_id, trajectory_id,
                 input_fingerprint, model_revision, config_digest, state, result_ref,
                 result_digest, provider_reservation_id FROM decisions WHERE execution_id = ?1",
                [reference.execution_id.as_str()],
                read_decision,
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(existing) = existing {
            if !same_decision(&existing.reference, reference) {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(existing);
        }
        ensure_current_lineage(&tx, &reference.lineage)?;
        let count = tx
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(schema::map_sqlite)?;
        if count >= MAX_DECISIONS {
            return Err(super::types::ExecutionStoreError::Capacity);
        }
        tx.execute(
            "INSERT INTO decisions(execution_id, run_id, episode_id, attempt_id, trajectory_id,
             input_fingerprint, model_revision, config_digest, state, result_ref, result_digest,
             provider_reservation_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL, NULL, NULL, ?9, ?9)",
            params![
                reference.execution_id,
                reference.lineage.run_id,
                reference.lineage.episode_id,
                reference.lineage.attempt_id,
                reference.lineage.trajectory_id,
                reference.input_fingerprint,
                reference.model_revision,
                reference.config_digest,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "decision",
            &reference.execution_id,
            "pending",
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.decision(&reference.execution_id)
    }

    /// Reserves provider budget using a unique provider execution identity. A restart can inspect
    /// this reservation and conservatively retain it as unknown instead of charging twice.
    pub fn reserve_provider(
        &mut self,
        reservation: &ProviderReservation,
    ) -> Result<ProviderReservation, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        reservation.validate()?;
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let decision = tx
            .query_row(
                "SELECT run_id, episode_id, attempt_id, trajectory_id, provider_reservation_id
                 FROM decisions WHERE execution_id = ?1",
                [reservation.execution_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if decision.0 != reservation.lineage.run_id
            || decision.1 != reservation.lineage.episode_id
            || decision.2 != reservation.lineage.attempt_id
            || decision.3 != reservation.lineage.trajectory_id
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if decision
            .4
            .as_deref()
            .is_some_and(|id| id != reservation.reservation_id)
        {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        let existing = tx
            .query_row(
                "SELECT reservation_id, execution_id, run_id, episode_id, attempt_id,
                 trajectory_id, provider_execution_id, reserved_units, actual_units, state,
                 failure_class
                 FROM provider_reservations WHERE reservation_id = ?1",
                [reservation.reservation_id.as_str()],
                read_reservation,
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if let Some(existing) = existing {
            if !same_reservation(&existing, reservation) {
                return Err(super::types::ExecutionStoreError::Conflict);
            }
            tx.commit().map_err(schema::map_sqlite)?;
            return Ok(existing);
        }
        let provider_identity = tx
            .query_row(
                "SELECT reservation_id FROM provider_reservations
                 WHERE provider_execution_id = ?1",
                [reservation.provider_execution_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(schema::map_sqlite)?;
        if provider_identity.is_some() {
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        tx.execute(
            "INSERT INTO provider_reservations(reservation_id, execution_id, run_id, episode_id,
             attempt_id, trajectory_id, provider_execution_id, reserved_units, actual_units,
             state, failure_class, result_ref, result_digest, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, NULL, NULL, NULL, ?10, ?10)",
            params![
                reservation.reservation_id,
                reservation.execution_id,
                reservation.lineage.run_id,
                reservation.lineage.episode_id,
                reservation.lineage.attempt_id,
                reservation.lineage.trajectory_id,
                reservation.provider_execution_id,
                i64::try_from(reservation.reserved_units).map_err(|_| {
                    super::types::ExecutionStoreError::InvalidProviderReservation
                })?,
                ProviderReservationState::Reserved.as_str(),
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        tx.execute(
            "UPDATE decisions SET provider_reservation_id = ?2, updated_at = ?3
             WHERE execution_id = ?1",
            params![reservation.execution_id, reservation.reservation_id, now],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "provider_reservation",
            &reservation.reservation_id,
            ProviderReservationState::Reserved.as_str(),
            None,
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.provider_reservation(&reservation.reservation_id)
    }
}
