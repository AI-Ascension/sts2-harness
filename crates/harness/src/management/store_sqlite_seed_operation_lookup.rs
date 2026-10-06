// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::super::super::contract_seed_v2::{
    SeedOperationPhaseV2, StoredSeedBindingV2, StoredSeedOperationV2,
    WORKFLOW_SEED_OPERATION_V2_SCHEMA,
};
use super::super::super::seed_v2_crypto::{validate_candidate_record, validate_operation_record};
use super::super::{SeedBindingLookup, SeedBindingRecord, StoreError};
use super::seed_binding::MAX_OPERATION_BYTES;
use super::support::{decode, sqlite_error};

pub(super) fn lookup_binding_tx(
    transaction: &rusqlite::Transaction<'_>,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) -> Result<SeedBindingLookup, StoreError> {
    let row = transaction
        .query_row(
            "SELECT workflow_run_id, actor_digest, request_digest, record
             FROM management_seed_bindings WHERE request_id = ?1",
            [request_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((stored_run_id, stored_actor, stored_digest, bytes)) = row else {
        let submission_exists = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM management_submissions WHERE request_id = ?1)",
                [request_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite_error)?
            != 0;
        return Ok(if submission_exists {
            SeedBindingLookup::Conflict
        } else {
            SeedBindingLookup::Missing
        });
    };
    if bytes.len() > MAX_OPERATION_BYTES {
        return Err(StoreError::new(
            "store_corrupt",
            "stored seed binding exceeds the supported bound",
        ));
    }
    let record = decode::<StoredSeedBindingV2>(&bytes)?;
    validate_candidate_record(&record)
        .map_err(|_| StoreError::new("store_corrupt", "stored seed binding is invalid"))?;
    if record.request_id != request_id
        || record.actor_digest != stored_actor
        || record.request_digest != stored_digest
        || record.workflow_run_id != stored_run_id
    {
        return Err(StoreError::new(
            "store_corrupt",
            "stored seed binding index does not match its record",
        ));
    }
    validate_run_indexes(
        transaction,
        &record.request_id,
        &record.request_digest,
        &record.workflow_run_id,
    )?;
    if stored_actor != actor_digest || stored_digest != request_digest {
        return Ok(SeedBindingLookup::Conflict);
    }
    Ok(SeedBindingLookup::Existing(Box::new(
        SeedBindingRecord::new(record),
    )))
}

pub(super) struct IndexedSeedOperation {
    pub(super) operation_id: String,
    pub(super) request_id: String,
    pub(super) workflow_run_id: String,
    pub(super) actor_digest: String,
    pub(super) request_digest: String,
    pub(super) configuration_digest: String,
    pub(super) phase: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn read_seed_operation_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<IndexedSeedOperation> {
    Ok(IndexedSeedOperation {
        operation_id: row.get(0)?,
        request_id: row.get(1)?,
        workflow_run_id: row.get(2)?,
        actor_digest: row.get(3)?,
        request_digest: row.get(4)?,
        configuration_digest: row.get(5)?,
        phase: row.get(6)?,
        bytes: row.get(7)?,
    })
}

pub(super) fn decode_seed_operation_indexed(
    row: IndexedSeedOperation,
) -> Result<StoredSeedOperationV2, StoreError> {
    if row.bytes.len() > MAX_OPERATION_BYTES {
        return Err(StoreError::new(
            "store_corrupt",
            "stored seed operation exceeds the supported bound",
        ));
    }
    let record = decode::<StoredSeedOperationV2>(&row.bytes)?;
    validate_operation_record(&record)
        .map_err(|_| StoreError::new("store_corrupt", "stored seed operation is invalid"))?;
    let phase = match record.phase {
        SeedOperationPhaseV2::KeyPinned => "key_pinned",
        SeedOperationPhaseV2::CandidatePersisted => "candidate_persisted",
    };
    if row.operation_id != record.operation_id
        || row.request_id != record.request_id
        || row.workflow_run_id != record.workflow_run_id
        || row.actor_digest != record.actor_digest
        || row.request_digest != record.request_digest
        || row.configuration_digest != record.configuration_digest
        || row.phase != phase
        || record.schema_version != WORKFLOW_SEED_OPERATION_V2_SCHEMA
    {
        return Err(StoreError::new(
            "store_corrupt",
            "seed operation columns do not match their closed record",
        ));
    }
    Ok(record)
}

pub(super) fn validate_candidate_run_rows(
    transaction: &rusqlite::Transaction<'_>,
    operation: &StoredSeedOperationV2,
) -> Result<(), StoreError> {
    validate_run_indexes(
        transaction,
        &operation.request_id,
        &operation.request_digest,
        &operation.workflow_run_id,
    )
}

fn validate_run_indexes(
    transaction: &rusqlite::Transaction<'_>,
    request_id: &str,
    request_digest: &str,
    workflow_run_id: &str,
) -> Result<(), StoreError> {
    let submission = transaction
        .query_row(
            "SELECT request_digest, workflow_run_id
             FROM management_submissions WHERE request_id = ?1",
            [request_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(sqlite_error)?;
    if !matches!(
        submission.as_ref(),
        Some((digest, run_id))
            if digest.as_str() == request_digest && run_id.as_str() == workflow_run_id
    ) {
        return Err(StoreError::new(
            "store_corrupt",
            "candidate-persisted operation has no matching submission row",
        ));
    }

    let run = transaction
        .query_row(
            "SELECT request_id, request_digest
             FROM management_runs WHERE workflow_run_id = ?1",
            [workflow_run_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(sqlite_error)?;
    if !matches!(
        run.as_ref(),
        Some((row_request_id, digest))
            if row_request_id.as_str() == request_id && digest.as_str() == request_digest
    ) {
        return Err(StoreError::new(
            "store_corrupt",
            "candidate-persisted operation has no matching workflow run row",
        ));
    }
    Ok(())
}

pub(super) fn same_seed_operation_request(
    proposed: &StoredSeedOperationV2,
    winner: &StoredSeedOperationV2,
) -> bool {
    proposed.schema_version == winner.schema_version
        && proposed.request_id == winner.request_id
        && proposed.actor_digest == winner.actor_digest
        && proposed.request_digest == winner.request_digest
        && proposed.workflow_run_id == winner.workflow_run_id
        && proposed.operation_id == winner.operation_id
        && proposed.mode == winner.mode
        && proposed.admitted_configuration == winner.admitted_configuration
        && proposed.configuration_digest == winner.configuration_digest
        && proposed.derivation.algorithm_id == winner.derivation.algorithm_id
}

pub(super) fn candidate_matches_operation(
    candidate: &StoredSeedBindingV2,
    operation: &StoredSeedOperationV2,
) -> bool {
    candidate.request_id == operation.request_id
        && candidate.actor_digest == operation.actor_digest
        && candidate.request_digest == operation.request_digest
        && candidate.workflow_run_id == operation.workflow_run_id
        && candidate.operation_id == operation.operation_id
        && candidate.mode == operation.mode
        && candidate.admitted_configuration == operation.admitted_configuration
        && candidate.configuration_digest == operation.configuration_digest
        && candidate.derivation.as_ref() == Some(&operation.derivation)
}
