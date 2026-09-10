// SPDX-License-Identifier: MIT

use rusqlite::params;
use sha2::{Digest, Sha256};

use super::schema;
use super::store_core::{ExecutionStore, append_event};
use super::store_provider::{
    MAX_DECISION_RESULT_BYTES, decision_query, read_decision, read_reservation,
};
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
            None,
        )
    }

    /// Completes a provider reservation and stores the exact validated result bytes in the same
    /// transaction as the completion state. The digest must be SHA-256 of those bytes.
    pub fn complete_provider_with_result(
        &mut self,
        reservation_id: &str,
        result_ref: &str,
        result_digest: &str,
        result_payload: &[u8],
        actual_units: u64,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.finish_provider(
            reservation_id,
            ProviderReservationState::Completed,
            None,
            result_ref,
            result_digest,
            Some(actual_units),
            Some(result_payload),
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
            None,
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
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_provider(
        &mut self,
        reservation_id: &str,
        state: ProviderReservationState,
        failure: Option<ProviderFailureClass>,
        result_ref: &str,
        result_digest: &str,
        actual_units: Option<u64>,
        result_payload: Option<&[u8]>,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if !valid_reference(reservation_id)
            || !valid_reference(result_ref)
            || !valid_reference(result_digest)
            || actual_units.is_some_and(|units| units == 0)
            || result_payload.is_some_and(|payload| !valid_result_payload(payload, result_digest))
            || state != ProviderReservationState::Completed && result_payload.is_some()
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
                let stored_result = tx
                    .query_row(
                        "SELECT result_ref, result_digest FROM provider_reservations
                         WHERE reservation_id = ?1",
                        [reservation_id],
                        |row| {
                            Ok((
                                row.get::<_, Option<String>>(0)?,
                                row.get::<_, Option<String>>(1)?,
                            ))
                        },
                    )
                    .map_err(schema::map_sqlite)?;
                if stored_result.0.as_deref() != Some(result_ref)
                    || stored_result.1.as_deref() != Some(result_digest)
                {
                    return Err(super::types::ExecutionStoreError::Conflict);
                }
                let query = decision_query("WHERE execution_id = ?1");
                let existing = tx
                    .query_row(&query, [current.execution_id.as_str()], read_decision)
                    .map_err(schema::map_sqlite)?;
                if existing.reference.result_ref.as_deref() != Some(result_ref)
                    || existing.reference.result_digest.as_deref() != Some(result_digest)
                    || existing.result_payload.as_deref() != result_payload
                {
                    return Err(super::types::ExecutionStoreError::Conflict);
                }
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(existing);
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
             result_payload = ?5, updated_at = ?6 WHERE execution_id = ?1",
            params![
                current.execution_id,
                decision_state,
                result_ref,
                result_digest,
                result_payload,
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

fn valid_result_payload(payload: &[u8], digest: &str) -> bool {
    !payload.is_empty()
        && payload.len() <= MAX_DECISION_RESULT_BYTES
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && format!("{:x}", Sha256::digest(payload)) == digest
}
