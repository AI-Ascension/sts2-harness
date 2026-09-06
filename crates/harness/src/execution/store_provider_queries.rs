// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::schema;
use super::store_core::ExecutionStore;
use super::types::{
    DecisionReference, ProviderFailureClass, ProviderReservation, ProviderReservationState,
    StoredDecision, valid_reference,
};

impl ExecutionStore {
    pub fn provider_reservation(
        &self,
        reservation_id: &str,
    ) -> Result<ProviderReservation, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
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
            })
    }

    /// Lists reservations whose provider usage is not conclusively accounted for. A restart must
    /// inspect these records before issuing any new inference request.
    pub fn pending_provider_reservations(
        &self,
        episode_id: &str,
    ) -> Result<Vec<ProviderReservation>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT reservation_id, execution_id, run_id, episode_id, attempt_id,
                 trajectory_id, provider_execution_id, reserved_units, actual_units, state,
                 failure_class FROM provider_reservations WHERE episode_id = ?1
                 AND state IN ('reserved', 'unknown') ORDER BY created_at, reservation_id",
            )
            .map_err(schema::map_sqlite)?;
        let rows = statement
            .query_map([episode_id], read_reservation)
            .map_err(schema::map_sqlite)?;
        rows.map(|row| row.map_err(schema::map_sqlite))
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn decision(
        &self,
        execution_id: &str,
    ) -> Result<StoredDecision, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        self.connection
            .query_row(
                "SELECT execution_id, run_id, episode_id, attempt_id, trajectory_id,
                 input_fingerprint, model_revision, config_digest, state, result_ref,
                 result_digest, provider_reservation_id FROM decisions WHERE execution_id = ?1",
                [execution_id],
                read_decision,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
                other => schema::map_sqlite(other),
            })
    }

    /// Returns a completed result only when every input/model/config fingerprint matches exactly.
    /// Pending or unknown provider calls are never eligible for reuse.
    pub fn reuse_completed_decision(
        &self,
        episode_id: &str,
        input_fingerprint: &str,
        model_revision: &str,
        config_digest: &str,
    ) -> Result<Option<StoredDecision>, super::types::ExecutionStoreError> {
        self.ensure_open()?;
        if [episode_id, input_fingerprint, model_revision, config_digest]
            .iter()
            .any(|value| !valid_reference(value))
        {
            return Err(super::types::ExecutionStoreError::InvalidDecision);
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT execution_id, run_id, episode_id, attempt_id, trajectory_id,
                 input_fingerprint, model_revision, config_digest, state, result_ref,
                 result_digest, provider_reservation_id FROM decisions WHERE episode_id = ?1
                 AND input_fingerprint = ?2 AND model_revision = ?3 AND config_digest = ?4
                 AND state = 'completed' AND result_ref IS NOT NULL AND result_digest IS NOT NULL
                 ORDER BY updated_at DESC, execution_id LIMIT 1",
            )
            .map_err(schema::map_sqlite)?;
        statement
            .query_row(
                params![episode_id, input_fingerprint, model_revision, config_digest],
                read_decision,
            )
            .optional()
            .map_err(schema::map_sqlite)
    }
}

pub(crate) fn same_decision(left: &DecisionReference, right: &DecisionReference) -> bool {
    left.lineage == right.lineage
        && left.execution_id == right.execution_id
        && left.input_fingerprint == right.input_fingerprint
        && left.model_revision == right.model_revision
        && left.config_digest == right.config_digest
}

pub(crate) fn same_reservation(left: &ProviderReservation, right: &ProviderReservation) -> bool {
    left.lineage == right.lineage
        && left.reservation_id == right.reservation_id
        && left.execution_id == right.execution_id
        && left.provider_execution_id == right.provider_execution_id
        && left.reserved_units == right.reserved_units
}

pub(crate) fn read_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredDecision> {
    let lineage = super::types::ExecutionLineage::new(
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
        row.get::<_, String>(4)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let reference = DecisionReference::new(
        lineage,
        row.get::<_, String>(0)?,
        row.get::<_, String>(5)?,
        row.get::<_, String>(6)?,
        row.get::<_, String>(7)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let state = row.get::<_, String>(8)?;
    if !matches!(
        state.as_str(),
        "pending" | "completed" | "failed" | "unknown"
    ) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let result_ref = row.get::<_, Option<String>>(9)?;
    let result_digest = row.get::<_, Option<String>>(10)?;
    if result_ref.is_some() != result_digest.is_some()
        || result_ref
            .as_deref()
            .is_some_and(|value| !super::types::valid_reference(value))
        || result_digest
            .as_deref()
            .is_some_and(|value| !super::types::valid_reference(value))
        || state == "pending" && (result_ref.is_some() || result_digest.is_some())
        || state == "completed" && result_ref.is_none()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(StoredDecision {
        reference: DecisionReference {
            result_ref,
            result_digest,
            ..reference
        },
        completed: state == "completed",
        unknown: state == "unknown",
        provider_reservation_id: row.get(11)?,
    })
}

pub(crate) fn read_reservation(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderReservation> {
    let lineage = super::types::ExecutionLineage::new(
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
        row.get::<_, String>(4)?,
        row.get::<_, String>(5)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let reserved_units =
        u64::try_from(row.get::<_, i64>(7)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let mut reservation = ProviderReservation::new(
        lineage,
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(6)?,
        reserved_units,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    reservation.actual_units = row
        .get::<_, Option<i64>>(8)?
        .map(u64::try_from)
        .transpose()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    reservation.state = ProviderReservationState::from_str(&row.get::<_, String>(9)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    reservation.failure = match row.get::<_, Option<String>>(10)? {
        Some(value) => Some(match value.as_str() {
            "authentication" => ProviderFailureClass::Authentication,
            "quota" => ProviderFailureClass::Quota,
            "outage" => ProviderFailureClass::Outage,
            "timeout" => ProviderFailureClass::Timeout,
            "incompatible_output" => ProviderFailureClass::IncompatibleOutput,
            "cancelled" => ProviderFailureClass::Cancelled,
            _ => return Err(rusqlite::Error::InvalidQuery),
        }),
        None => None,
    };
    reservation
        .validate()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(reservation)
}
