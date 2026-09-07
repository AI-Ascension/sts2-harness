// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::store_provider::read_reservation;
use super::types::{
    ProviderFailureClass, ProviderReservationState, StoredDecision, valid_reference,
};

impl ExecutionStore {
    pub fn complete_provider(
        &mut self,
        reservation_id: &str,
        result_ref: &str,
        result_digest: &str,
        actual_units: u64,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.finish_provider(
            reservation_id,
            ProviderReservationState::Completed,
            None,
            result_ref,
            result_digest,
            Some(actual_units),
        )
    }

    pub fn fail_provider(
        &mut self,
        reservation_id: &str,
        failure: ProviderFailureClass,
        actual_units: Option<u64>,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.finish_provider(
            reservation_id,
            ProviderReservationState::Failed,
            Some(failure),
            "provider-failure",
            "provider-failure",
            actual_units,
        )
    }

    pub fn mark_provider_unknown(
        &mut self,
        reservation_id: &str,
        failure: ProviderFailureClass,
        actual_units: Option<u64>,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.finish_provider(
            reservation_id,
            ProviderReservationState::Unknown,
            Some(failure),
            "provider-unknown",
            "provider-unknown",
            actual_units,
        )
    }

    fn finish_provider(
        &mut self,
        reservation_id: &str,
        state: ProviderReservationState,
        failure: Option<ProviderFailureClass>,
        result_ref: &str,
        result_digest: &str,
        actual_units: Option<u64>,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_reference(reservation_id)
            || !valid_reference(result_ref)
            || !valid_reference(result_digest)
            || actual_units.is_some_and(|units| units == 0)
        {
            return Err(super::types::ExecutionStoreError::InvalidProviderReservation);
        }
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        let current = tx
            .query_row(
                "SELECT reservation_id, execution_id, run_id, episode_id, attempt_id,
                 trajectory_id, provider_execution_id, reserved_units, actual_units, state,
                 failure_class
                 FROM provider_reservations WHERE reservation_id = ?1",
                [reservation_id],
                read_reservation,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })?;
        if current.state != ProviderReservationState::Reserved {
            if current.state == state
                && current.actual_units == actual_units
                && current.failure == failure
            {
                tx.commit().map_err(schema::map_sqlite)?;
                return self.decision(&current.execution_id);
            }
            return Err(super::types::ExecutionStoreError::Conflict);
        }
        if actual_units.is_some_and(|units| units > current.reserved_units) {
            return Err(super::types::ExecutionStoreError::InvalidProviderReservation);
        }
        if matches!(state, ProviderReservationState::Completed)
            && (failure.is_some() || actual_units.is_none())
            || matches!(
                state,
                ProviderReservationState::Failed | ProviderReservationState::Unknown
            ) && failure.is_none()
        {
            return Err(super::types::ExecutionStoreError::InvalidProviderReservation);
        }
        tx.execute(
            "UPDATE provider_reservations SET state = ?2, failure_class = ?3,
             actual_units = ?4, result_ref = ?5, result_digest = ?6, updated_at = ?7
             WHERE reservation_id = ?1",
            params![
                reservation_id,
                state.as_str(),
                failure.map(ProviderFailureClass::as_str),
                actual_units.map(i64::try_from).transpose().map_err(|_| {
                    super::types::ExecutionStoreError::InvalidProviderReservation
                })?,
                result_ref,
                result_digest,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        let decision_state = match state {
            ProviderReservationState::Completed => "completed",
            ProviderReservationState::Unknown => "unknown",
            ProviderReservationState::Failed => "failed",
            ProviderReservationState::Reserved => "pending",
        };
        tx.execute(
            "UPDATE decisions SET state = ?2, result_ref = ?3, result_digest = ?4,
             updated_at = ?5 WHERE execution_id = ?1",
            params![
                current.execution_id,
                decision_state,
                result_ref,
                result_digest,
                now
            ],
        )
        .map_err(schema::map_sqlite)?;
        append_event(
            &tx,
            "provider_reservation",
            reservation_id,
            state.as_str(),
            Some(result_digest),
            now,
        )?;
        tx.commit().map_err(schema::map_sqlite)?;
        self.decision(&current.execution_id)
    }
}
