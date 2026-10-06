// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::super::super::contract_seed_v2::{SeedBindingStateV2, StoredSeedBindingV2};
use super::super::super::seed_v2_crypto::validate_candidate_record;
use super::super::{SeedBindingLookup, SeedBindingRecord, StoreError};
use super::SqliteWorkflowStore;
use super::support::{connection, decode, encode, sqlite_error};

pub(super) const MAX_OPERATION_BYTES: usize = 8192;

pub(super) fn lookup(
    store: &SqliteWorkflowStore,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) -> Result<SeedBindingLookup, StoreError> {
    let connection = connection(store)?;
    let row = connection
        .query_row(
            "SELECT actor_digest, request_digest, record
             FROM management_seed_bindings WHERE request_id = ?1",
            [request_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((stored_actor, stored_digest, bytes)) = row else {
        let submission_exists = connection
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
    if stored_actor != actor_digest || stored_digest != request_digest {
        return Ok(SeedBindingLookup::Conflict);
    }
    let record = decode::<StoredSeedBindingV2>(&bytes)?;
    validate_candidate_record(&record)
        .map_err(|_| StoreError::new("store_corrupt", "stored seed binding is invalid"))?;
    if record.request_id != request_id
        || record.actor_digest != stored_actor
        || record.request_digest != stored_digest
    {
        return Err(StoreError::new(
            "store_corrupt",
            "stored seed binding index does not match its record",
        ));
    }
    Ok(SeedBindingLookup::Existing(Box::new(
        SeedBindingRecord::new(record),
    )))
}

pub(super) fn read(
    store: &SqliteWorkflowStore,
    workflow_run_id: &str,
) -> Result<Option<SeedBindingRecord>, StoreError> {
    let connection = connection(store)?;
    let bytes = connection
        .query_row(
            "SELECT record FROM management_seed_bindings WHERE workflow_run_id = ?1",
            [workflow_run_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let record = decode::<StoredSeedBindingV2>(&bytes)?;
    validate_candidate_record(&record)
        .map_err(|_| StoreError::new("store_corrupt", "stored seed binding is invalid"))?;
    if record.workflow_run_id != workflow_run_id {
        return Err(StoreError::new(
            "store_corrupt",
            "stored seed binding run identity does not match its index",
        ));
    }
    Ok(Some(SeedBindingRecord::new(record)))
}

pub(super) fn mark_awaiting_host_context(
    store: &SqliteWorkflowStore,
    workflow_run_id: &str,
    actor_digest: &str,
    request_digest: &str,
) -> Result<SeedBindingRecord, StoreError> {
    let mut connection = connection(store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let bytes = transaction
        .query_row(
            "SELECT record FROM management_seed_bindings WHERE workflow_run_id = ?1",
            [workflow_run_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or_else(|| StoreError::new("seed_binding_not_found", "seed binding was not found"))?;
    let mut record = decode::<StoredSeedBindingV2>(&bytes)?;
    validate_candidate_record(&record)
        .map_err(|_| StoreError::new("store_corrupt", "stored seed binding is invalid"))?;
    if record.workflow_run_id != workflow_run_id
        || record.actor_digest != actor_digest
        || record.request_digest != request_digest
    {
        return Err(StoreError::new(
            "seed_binding_conflict",
            "seed binding identity does not match the requested reservation",
        ));
    }
    if record.state == SeedBindingStateV2::CandidatePersisted {
        record.state = SeedBindingStateV2::AwaitingHostContext;
        let encoded = encode(&record)?;
        transaction
            .execute(
                "UPDATE management_seed_bindings SET record = ?1
                 WHERE workflow_run_id = ?2 AND actor_digest = ?3 AND request_digest = ?4",
                params![encoded, workflow_run_id, actor_digest, request_digest],
            )
            .map_err(sqlite_error)?;
    }
    transaction.commit().map_err(sqlite_error)?;
    Ok(SeedBindingRecord::new(record))
}
