// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::super::contract_seed_v2::{SeedOperationPhaseV2, StoredSeedOperationV2};
use super::super::super::seed_v2_crypto::validate_operation_record;
use super::super::{SeedBindingLookup, SeedOperationLookup, SeedOperationRecord, StoreError};
use super::SqliteWorkflowStore;
use super::seed_binding::MAX_OPERATION_BYTES;
use super::seed_operation_lookup::{
    candidate_matches_operation, decode_seed_operation_indexed, lookup_binding_tx,
    read_seed_operation_row, same_seed_operation_request, validate_candidate_run_rows,
};
use super::support::{connection, encode, sqlite_error};

pub(super) fn lookup(
    store: &SqliteWorkflowStore,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) -> Result<SeedOperationLookup, StoreError> {
    let mut connection = connection(store)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    let result = lookup_in_transaction(&transaction, request_id, actor_digest, request_digest)?;
    transaction.commit().map_err(sqlite_error)?;
    Ok(result)
}

pub(super) fn lookup_in_transaction(
    transaction: &Transaction<'_>,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) -> Result<SeedOperationLookup, StoreError> {
    let row = transaction
        .query_row(
            "SELECT operation_id, request_id, workflow_run_id, actor_digest,
                    request_digest, configuration_digest, phase, record
             FROM management_seed_operations WHERE request_id = ?1",
            [request_id],
            read_seed_operation_row,
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some(row) = row else {
        return lookup_legacy_or_missing_tx(transaction, request_id, actor_digest, request_digest);
    };
    let record = decode_seed_operation_indexed(row)?;
    if record.actor_digest != actor_digest || record.request_digest != request_digest {
        return Ok(SeedOperationLookup::Conflict);
    }
    match record.phase {
        SeedOperationPhaseV2::KeyPinned => {
            let related_rows = transaction
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM management_submissions WHERE request_id = ?1
                         UNION ALL SELECT 1 FROM management_runs WHERE workflow_run_id = ?2
                         UNION ALL SELECT 1 FROM management_seed_bindings WHERE request_id = ?1
                     )",
                    params![record.request_id, record.workflow_run_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(sqlite_error)?
                != 0;
            if related_rows {
                return Err(StoreError::new(
                    "store_corrupt",
                    "key-pinned seed operation unexpectedly has candidate rows",
                ));
            }
            Ok(SeedOperationLookup::Prepared(Box::new(
                SeedOperationRecord::new(record),
            )))
        }
        SeedOperationPhaseV2::CandidatePersisted => {
            validate_candidate_run_rows(transaction, &record)?;
            let binding =
                match lookup_binding_tx(transaction, request_id, actor_digest, request_digest)? {
                    SeedBindingLookup::Existing(binding) => *binding,
                    SeedBindingLookup::Missing | SeedBindingLookup::Conflict => {
                        return Err(StoreError::new(
                            "store_corrupt",
                            "candidate-persisted operation has no matching seed binding",
                        ));
                    }
                };
            if !candidate_matches_operation(binding.record(), &record) {
                return Err(StoreError::new(
                    "store_corrupt",
                    "candidate seed binding differs from its operation reservation",
                ));
            }
            Ok(SeedOperationLookup::CandidatePersisted(Box::new(binding)))
        }
    }
}

pub(super) fn read_candidate_operation_in_transaction(
    transaction: &Transaction<'_>,
    proposed: &StoredSeedOperationV2,
) -> Result<StoredSeedOperationV2, StoreError> {
    let row = transaction
        .query_row(
            "SELECT operation_id, request_id, workflow_run_id, actor_digest,
                    request_digest, configuration_digest, phase, record
             FROM management_seed_operations WHERE operation_id = ?1",
            [&proposed.operation_id],
            read_seed_operation_row,
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or_else(|| {
            StoreError::new(
                "seed_operation_conflict",
                "candidate seed operation reservation is missing",
            )
        })?;
    let stored = decode_seed_operation_indexed(row)?;
    if stored.phase != SeedOperationPhaseV2::KeyPinned
        || stored.operation_id != proposed.operation_id
        || stored.request_id != proposed.request_id
        || stored.workflow_run_id != proposed.workflow_run_id
        || stored.actor_digest != proposed.actor_digest
        || stored.request_digest != proposed.request_digest
    {
        return Err(StoreError::new(
            "seed_operation_conflict",
            "candidate seed operation does not match its committed reservation",
        ));
    }
    Ok(stored)
}

fn lookup_legacy_or_missing_tx(
    transaction: &Transaction<'_>,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) -> Result<SeedOperationLookup, StoreError> {
    match lookup_binding_tx(transaction, request_id, actor_digest, request_digest)? {
        SeedBindingLookup::Existing(binding) => {
            // Completed v2 rows written before the additive operation table are
            // already durable key pins; replay them unchanged without backfill.
            return Ok(SeedOperationLookup::CandidatePersisted(binding));
        }
        SeedBindingLookup::Conflict => return Ok(SeedOperationLookup::Conflict),
        SeedBindingLookup::Missing => {}
    }
    let submission_exists = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM management_submissions WHERE request_id = ?1)",
            [request_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?
        != 0;
    Ok(if submission_exists {
        SeedOperationLookup::Conflict
    } else {
        SeedOperationLookup::Missing
    })
}

pub(super) fn reserve(
    store: &SqliteWorkflowStore,
    proposed: SeedOperationRecord,
) -> Result<SeedOperationLookup, StoreError> {
    let proposed = proposed.into_record();
    validate_operation_record(&proposed).map_err(|_| {
        StoreError::new("seed_operation_invalid", "seed operation record is invalid")
    })?;
    if proposed.phase != SeedOperationPhaseV2::KeyPinned {
        return Err(StoreError::new(
            "seed_operation_invalid",
            "new seed operation reservations must be key-pinned",
        ));
    }
    let mut connection = connection(store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let existing = transaction
        .query_row(
            "SELECT operation_id, request_id, workflow_run_id, actor_digest,
                    request_digest, configuration_digest, phase, record
             FROM management_seed_operations
             WHERE request_id = ?1 OR operation_id = ?2 OR workflow_run_id = ?3
             LIMIT 1",
            params![
                proposed.request_id,
                proposed.operation_id,
                proposed.workflow_run_id
            ],
            read_seed_operation_row,
        )
        .optional()
        .map_err(sqlite_error)?;
    if let Some(row) = existing {
        let winner = decode_seed_operation_indexed(row)?;
        if !same_seed_operation_request(&proposed, &winner) {
            return Ok(SeedOperationLookup::Conflict);
        }
        let result = lookup_in_transaction(
            &transaction,
            &winner.request_id,
            &winner.actor_digest,
            &winner.request_digest,
        )?;
        transaction.commit().map_err(sqlite_error)?;
        return Ok(result);
    }
    let collision = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM management_submissions WHERE request_id = ?1
                UNION ALL SELECT 1 FROM management_runs WHERE workflow_run_id = ?2
            )",
            params![proposed.request_id, proposed.workflow_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?
        != 0;
    if collision {
        return Ok(SeedOperationLookup::Conflict);
    }
    let bytes = encode(&proposed)?;
    if bytes.len() > MAX_OPERATION_BYTES {
        return Err(StoreError::new(
            "seed_operation_too_large",
            "seed operation reservation exceeds the supported bound",
        ));
    }
    transaction
        .execute(
            "INSERT INTO management_seed_operations(
                operation_id, request_id, workflow_run_id, actor_digest, request_digest,
                configuration_digest, phase, record
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'key_pinned', ?7)",
            params![
                proposed.operation_id,
                proposed.request_id,
                proposed.workflow_run_id,
                proposed.actor_digest,
                proposed.request_digest,
                proposed.configuration_digest,
                bytes,
            ],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)?;
    drop(connection);
    lookup(
        store,
        &proposed.request_id,
        &proposed.actor_digest,
        &proposed.request_digest,
    )
}
