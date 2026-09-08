// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::types::{
    StoredWorkerHandoff, WorkerControlMode, WorkerControlState, WorkerHandoffState,
    WorkerReservationState, WorkerTerminalReceipt, WorkerTuple, valid_digest,
    valid_worker_existing_id, valid_worker_identity, valid_worker_uuid4,
};
use crate::worker_handoff::TerminalRecord;

pub(crate) fn worker_handoff_select(suffix: &str) -> String {
    format!(
        "SELECT handoff_id, deployment_id, job_id, attempt_id, attempt_number,
         worker_owner_id, worker_profile_digest, run_id, episode_id, trajectory_id,
         payload_digest, worker_boot_id, watchdog_boot_id, mode_sequence, state,
         reservation_state, terminal_status, terminal_ref, checkpoint_sequence,
         result_digest, terminal_record, acknowledged, ack_digest FROM worker_handoffs {suffix}"
    )
}

pub(crate) fn read_control(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkerControlState> {
    if row.get::<_, i64>(0)? != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let deployment_id = row.get::<_, String>(1)?;
    let worker_owner_id = row.get::<_, String>(2)?;
    let worker_profile_digest = row.get::<_, String>(3)?;
    let worker_boot_id = row.get::<_, String>(4)?;
    let watchdog_boot_id = row.get::<_, Option<String>>(5)?;
    if !valid_worker_identity(&deployment_id)
        || !valid_worker_identity(&worker_owner_id)
        || !valid_digest(&worker_profile_digest)
        || !valid_worker_uuid4(&worker_boot_id)
        || watchdog_boot_id
            .as_deref()
            .is_some_and(|id| !valid_worker_uuid4(id))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mode = WorkerControlMode::from_str(&row.get::<_, String>(6)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let mode_sequence = bounded_u64(row.get::<_, i64>(7)?)?;
    let generation = bounded_u64(row.get::<_, i64>(8)?)?;
    if generation == 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let authenticated = read_bool(row.get::<_, i64>(9)?)?;
    let admitting = read_bool(row.get::<_, i64>(10)?)?;
    if admitting != (authenticated && mode.admits())
        || mode_sequence == 0 && authenticated
        || watchdog_boot_id.is_none() != !authenticated
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(WorkerControlState {
        deployment_id,
        worker_owner_id,
        worker_profile_digest,
        worker_boot_id,
        watchdog_boot_id,
        mode,
        mode_sequence,
        generation,
        authenticated,
        admitting,
    })
}

pub(crate) fn read_handoff(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredWorkerHandoff> {
    let tuple = WorkerTuple::new(
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
        bounded_u64(row.get::<_, i64>(4)?)?,
        row.get::<_, String>(5)?,
        row.get::<_, String>(6)?,
        row.get::<_, String>(7)?,
        row.get::<_, String>(8)?,
        row.get::<_, String>(9)?,
        row.get::<_, String>(10)?,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let worker_boot_id = row.get::<_, String>(11)?;
    let watchdog_boot_id = row.get::<_, String>(12)?;
    let mode_sequence = bounded_u64(row.get::<_, i64>(13)?)?;
    if !valid_worker_uuid4(&worker_boot_id)
        || !valid_worker_uuid4(&watchdog_boot_id)
        || mode_sequence == 0
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let state = WorkerHandoffState::from_str(&row.get::<_, String>(14)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    let reservation_state = WorkerReservationState::from_str(&row.get::<_, String>(15)?)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    if matches!(state, WorkerHandoffState::Unknown)
        != matches!(reservation_state, WorkerReservationState::Unknown)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let terminal = read_terminal(
        &tuple,
        row.get::<_, Option<String>>(16)?,
        row.get::<_, Option<String>>(17)?,
        row.get::<_, Option<i64>>(18)?,
        row.get::<_, Option<String>>(19)?,
        row.get::<_, Option<Vec<u8>>>(20)?,
    )?;
    if matches!(
        state,
        WorkerHandoffState::Terminal | WorkerHandoffState::Acknowledged
    ) != terminal.is_some()
        || terminal.is_some() && reservation_state != WorkerReservationState::Reserved
        || terminal.is_none()
            && !matches!(state, WorkerHandoffState::Unknown)
            && reservation_state != WorkerReservationState::Reserved
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let acknowledged = read_bool(row.get::<_, i64>(21)?)?;
    let acknowledgment_digest = row.get::<_, Option<String>>(22)?;
    if acknowledged != matches!(state, WorkerHandoffState::Acknowledged)
        || acknowledged != acknowledgment_digest.is_some()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if let (Some(receipt), Some(digest)) = (&terminal, &acknowledgment_digest)
        && (!valid_digest(digest)
            || receipt.acknowledgment_digest().as_deref() != Ok(digest.as_str()))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(StoredWorkerHandoff {
        tuple,
        worker_boot_id,
        watchdog_boot_id,
        mode_sequence,
        state,
        reservation_state,
        terminal,
        acknowledged,
        acknowledgment_digest,
    })
}

fn read_terminal(
    tuple: &WorkerTuple,
    status: Option<String>,
    terminal_ref: Option<String>,
    checkpoint_sequence: Option<i64>,
    result_digest: Option<String>,
    terminal_record: Option<Vec<u8>>,
) -> rusqlite::Result<Option<WorkerTerminalReceipt>> {
    match (
        status,
        terminal_ref,
        checkpoint_sequence,
        result_digest,
        terminal_record,
    ) {
        (None, None, None, None, None) => Ok(None),
        (
            Some(status),
            Some(terminal_ref),
            Some(checkpoint_sequence),
            Some(result_digest),
            Some(bytes),
        ) => {
            let decoded =
                TerminalRecord::decode(&bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let receipt = WorkerTerminalReceipt::from_terminal(tuple.clone(), &decoded)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            if receipt.canonical_bytes() != bytes.as_slice()
                || receipt.status.as_str() != status
                || receipt.terminal_ref != terminal_ref
                || receipt.checkpoint_sequence != bounded_u64(checkpoint_sequence)?
                || receipt.result_digest != result_digest
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok(Some(receipt))
        }
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn bounded_u64(value: i64) -> rusqlite::Result<u64> {
    let value = u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)?;
    if value > super::types::WORKER_MAX_ATTEMPT_NUMBER {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(value)
}

fn read_bool(value: i64) -> rusqlite::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(super) fn ensure_dispatch_authority(
    tx: &rusqlite::Transaction<'_>,
    context: &super::types::WorkerAdmissionContext,
    tuple: &super::types::WorkerTuple,
) -> Result<(), super::types::ExecutionStoreError> {
    let control = tx
        .query_row(
            super::store_worker_control::CONTROL_SELECT,
            [],
            read_control,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
            other => super::schema::map_sqlite(other),
        })?;
    if control.deployment_id != tuple.deployment_id
        || control.worker_owner_id != tuple.worker_owner_id
        || control.worker_profile_digest != tuple.worker_profile_digest
        || control.worker_boot_id != context.worker_boot_id
        || control.watchdog_boot_id.as_deref() != Some(context.watchdog_boot_id.as_str())
        || control.mode != super::types::WorkerControlMode::Running
        || !control.authenticated
        || !control.admitting
        || control.mode_sequence != context.mode_sequence
    {
        return Err(super::types::ExecutionStoreError::Conflict);
    }
    Ok(())
}

pub(super) fn read_handoff_by_id(
    tx: &rusqlite::Transaction<'_>,
    handoff_id: &str,
) -> Result<Option<super::types::StoredWorkerHandoff>, super::types::ExecutionStoreError> {
    tx.query_row(
        &worker_handoff_select("WHERE handoff_id = ?1"),
        [handoff_id],
        read_handoff,
    )
    .optional()
    .map_err(super::schema::map_sqlite)
}

pub(super) fn read_job(
    tx: &rusqlite::Transaction<'_>,
    job_id: &str,
) -> Result<
    (String, String, super::types::JobState, Option<String>),
    super::types::ExecutionStoreError,
> {
    tx.query_row(
        "SELECT episode_id, payload_digest, state, claim_token FROM jobs WHERE job_id = ?1",
        [job_id],
        |row| {
            let state = super::types::JobState::from_str(&row.get::<_, String>(2)?)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            let episode_id = row.get::<_, String>(0)?;
            let payload_digest = row.get::<_, String>(1)?;
            let claim_token = row.get::<_, Option<String>>(3)?;
            if !valid_worker_existing_id(&episode_id)
                || !valid_digest(&payload_digest)
                || claim_token
                    .as_deref()
                    .is_some_and(|value| !valid_worker_existing_id(value))
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            Ok((episode_id, payload_digest, state, claim_token))
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => super::types::ExecutionStoreError::Missing,
        other => super::schema::map_sqlite(other),
    })
}

pub(super) fn read_existing_completion(
    tx: &rusqlite::Transaction<'_>,
    episode_id: &str,
) -> Result<Option<super::types::CompletionRecord>, super::types::ExecutionStoreError> {
    tx.query_row(
        "SELECT run_id, episode_id, attempt_id, trajectory_id, status, terminal_ref,
         checkpoint_sequence, result_digest FROM completions WHERE episode_id = ?1",
        [episode_id],
        super::store_completion::read_completion,
    )
    .optional()
    .map_err(super::schema::map_sqlite)
}
