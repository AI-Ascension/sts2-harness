// SPDX-License-Identifier: MIT

use rusqlite::{TransactionBehavior, params};

use super::super::super::super::contract::{MAX_EVENTS_PER_RUN, RunEvent, RunSnapshot};
use super::super::super::super::contract_seed_v2::{
    SeedBindingStateV2, SeedModeV2, SeedOperationPhaseV2, StoredSeedBindingV2,
    StoredSeedOperationV2,
};
use super::super::super::super::seed_v2_crypto::{
    validate_candidate_record, validate_operation_record,
};
use super::super::super::{SeedOperationRecord, StoreError};
use super::super::SqliteWorkflowStore;
use super::super::support::{connection, encode, insert_event, sqlite_error, to_i64};
use crate::management::store::ops::validate_initial_run;

pub(crate) fn create_run(
    store: &SqliteWorkflowStore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
    initial_events: Vec<RunEvent>,
) -> Result<(), StoreError> {
    create_run_inner(
        store,
        request_id,
        request_digest,
        snapshot,
        initial_events,
        None,
        None,
    )
}

pub(crate) fn create_seeded_run(
    store: &SqliteWorkflowStore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
    initial_events: Vec<RunEvent>,
    operation: Option<SeedOperationRecord>,
    seed_binding: StoredSeedBindingV2,
) -> Result<(), StoreError> {
    create_run_inner(
        store,
        request_id,
        request_digest,
        snapshot,
        initial_events,
        Some(seed_binding),
        operation.map(SeedOperationRecord::into_record),
    )
}

fn create_run_inner(
    store: &SqliteWorkflowStore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
    initial_events: Vec<RunEvent>,
    seed_binding: Option<StoredSeedBindingV2>,
    mut operation: Option<StoredSeedOperationV2>,
) -> Result<(), StoreError> {
    validate_initial_run(&snapshot, &initial_events)?;
    if let Some(record) = seed_binding.as_ref() {
        validate_candidate_record(record).map_err(|_| {
            StoreError::new("seed_binding_invalid", "seed binding record is invalid")
        })?;
        if record.request_id != request_id
            || record.request_digest != request_digest
            || record.workflow_run_id != snapshot.workflow_run_id
            || snapshot.admission.as_ref() != Some(&record.admitted_configuration)
            || record.state != SeedBindingStateV2::CandidatePersisted
        {
            return Err(StoreError::new(
                "seed_binding_identity_conflict",
                "seed binding record does not match its initial workflow reservation",
            ));
        }
    }
    let needs_operation = seed_binding
        .as_ref()
        .is_some_and(|record| record.mode == SeedModeV2::DeriveOnce);
    if needs_operation != operation.is_some() {
        return Err(StoreError::new(
            "seed_operation_required",
            "derive-once candidate requires its committed operation reservation",
        ));
    }
    if let (Some(binding), Some(record)) = (seed_binding.as_ref(), operation.as_ref()) {
        validate_operation_record(record).map_err(|_| {
            StoreError::new("seed_operation_invalid", "seed operation record is invalid")
        })?;
        if record.phase != SeedOperationPhaseV2::KeyPinned
            || record.request_id != request_id
            || record.request_digest != request_digest
            || record.workflow_run_id != snapshot.workflow_run_id
            || record.operation_id != binding.operation_id
            || record.actor_digest != binding.actor_digest
            || record.admitted_configuration != binding.admitted_configuration
            || record.configuration_digest != binding.configuration_digest
            || binding.derivation.as_ref() != Some(&record.derivation)
        {
            return Err(StoreError::new(
                "seed_operation_conflict",
                "candidate does not match its immutable seed operation reservation",
            ));
        }
    }
    let initial_events = initial_events
        .into_iter()
        .map(|event| {
            event
                .seal_integrity()
                .map_err(|error| StoreError::new("event_integrity", error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if initial_events.len() > MAX_EVENTS_PER_RUN {
        return Err(StoreError::new(
            "event_limit",
            "initial workflow event set exceeds the supported bound",
        ));
    }
    let snapshot_bytes = encode(&snapshot)?;
    let mut connection = connection(store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let operation_collision = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM management_seed_operations
                 WHERE request_id = ?1 OR workflow_run_id = ?2
            )",
            params![request_id, snapshot.workflow_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?
        != 0;
    match operation.as_ref() {
        Some(proposed) => {
            let stored = super::super::seed_operation::read_candidate_operation_in_transaction(
                &transaction,
                proposed,
            )?;
            if stored != *proposed {
                return Err(StoreError::new(
                    "seed_operation_conflict",
                    "candidate seed operation differs from its committed reservation",
                ));
            }
        }
        None if operation_collision => {
            return Err(StoreError::new(
                "seed_operation_conflict",
                "workflow request or run is reserved by a seed operation",
            ));
        }
        None => {}
    }
    let duplicate = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM management_submissions WHERE request_id = ?1
                UNION ALL
                SELECT 1 FROM management_runs WHERE workflow_run_id = ?2
            )",
            params![request_id, snapshot.workflow_run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite_error)?
        != 0;
    if duplicate {
        return Err(StoreError::new(
            "duplicate_run",
            "workflow run or submission already exists",
        ));
    }
    transaction
        .execute(
            "INSERT INTO management_submissions(request_id, request_digest, workflow_run_id)
             VALUES (?1, ?2, ?3)",
            params![request_id, request_digest, snapshot.workflow_run_id],
        )
        .map_err(sqlite_error)?;
    transaction
        .execute(
            "INSERT INTO management_runs(
                workflow_run_id, request_id, request_digest, snapshot, oldest_sequence
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                snapshot.workflow_run_id,
                request_id,
                request_digest,
                snapshot_bytes,
                to_i64(initial_events[0].sequence, "oldest event sequence")?
            ],
        )
        .map_err(sqlite_error)?;
    for event in &initial_events {
        insert_event(&transaction, event)?;
    }
    if let Some(record) = seed_binding.as_ref() {
        let bytes = encode(record)?;
        if bytes.len() > 8192 {
            return Err(StoreError::new(
                "seed_binding_too_large",
                "seed binding record exceeds the supported bound",
            ));
        }
        transaction
            .execute(
                "INSERT INTO management_seed_bindings(
                    workflow_run_id, request_id, actor_digest, request_digest, record
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    record.workflow_run_id,
                    record.request_id,
                    record.actor_digest,
                    record.request_digest,
                    bytes,
                ],
            )
            .map_err(sqlite_error)?;
    }
    if let Some(record) = operation.as_mut() {
        record.phase = SeedOperationPhaseV2::CandidatePersisted;
        let bytes = encode(record)?;
        let changed = transaction
            .execute(
                "UPDATE management_seed_operations
                 SET phase = 'candidate_persisted', record = ?1
                 WHERE operation_id = ?2 AND request_id = ?3 AND workflow_run_id = ?4
                   AND actor_digest = ?5 AND request_digest = ?6
                   AND configuration_digest = ?7 AND phase = 'key_pinned'",
                params![
                    bytes,
                    record.operation_id,
                    record.request_id,
                    record.workflow_run_id,
                    record.actor_digest,
                    record.request_digest,
                    record.configuration_digest,
                ],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(StoreError::new(
                "seed_operation_conflict",
                "seed operation reservation changed before candidate commit",
            ));
        }
    }
    transaction.commit().map_err(sqlite_error)
}
