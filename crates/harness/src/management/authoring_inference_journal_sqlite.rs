// SPDX-License-Identifier: MIT

//! Durable journal for authoring-inference operations.
//!
//! Requirement 4 of `sts2-harness#105` says a reservation is *persisted*, and
//! acceptance criterion 4 says a restart preserves one operation and an honest
//! cost/outcome state. The in-memory journal satisfies both only to the depth
//! of a fresh service over the same in-process map, so a proposal could be
//! replayed differently after the owning process exits. This implementation
//! keeps the reservation and its terminal outcome in the same database the
//! served composition already opens, so "restart" means across a reopened store
//! rather than across a re-composition inside one process.
//!
//! The store is held behind an `Arc` because the served composition passes the
//! same database to the run store and to this journal; both must observe one
//! connection so the terminal write and its read-back are one boundary.
//!
//! That boundary is a real transaction, not the shared `Mutex` alone. The mutex
//! serializes callers inside one process, but the durable journal's stated
//! purpose is to survive a restart, and a restart is exactly the case where a
//! second process, or a second `SqliteWorkflowStore` over the same file, holds a
//! different connection: a competing statement can land between this module's
//! write and its read-back. `TransactionBehavior::Immediate` takes the write
//! lock up front, so the reservation row and the read that classifies it cannot
//! be separated by another writer. This follows
//! `inference_profile_revision_sqlite.rs`, the sibling this module already names
//! as its pattern.

use std::sync::Arc;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::super::super::contract_authoring_inference::{
    AUTHORING_INFERENCE_OPERATION_SCHEMA_VERSION, AuthoringInferenceCost,
    AuthoringInferenceOperationRecord, AuthoringInferenceOperationState,
    AuthoringInferenceProposal,
};
use super::super::super::store::{SqliteWorkflowStore, StoreError};
use super::{
    AuthoringInferenceBegin, AuthoringInferenceJournal, bounded_detail, validate_identity,
};

/// Durable journal over the shared management SQLite store.
pub struct SqliteAuthoringInferenceJournal {
    store: Arc<SqliteWorkflowStore>,
}

impl SqliteAuthoringInferenceJournal {
    #[must_use]
    pub fn new(store: Arc<SqliteWorkflowStore>) -> Self {
        Self { store }
    }
}

impl AuthoringInferenceJournal for SqliteAuthoringInferenceJournal {
    fn begin(
        &self,
        operation_id: &str,
        draft_id: &str,
        client_mutation_id: &str,
        request_digest: &str,
        reserved: AuthoringInferenceCost,
    ) -> Result<AuthoringInferenceBegin, StoreError> {
        validate_identity(draft_id, client_mutation_id, request_digest)?;
        let mut connection = self.store.connection.lock().map_err(|_| poisoned())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        if let Some(existing) = read_record(&transaction, operation_id)? {
            // Nothing was written on this path, but the transaction still has to
            // be closed rather than dropped: `Immediate` has already taken the
            // write lock, and a dropped transaction rolls back and releases it.
            transaction.commit().map_err(sqlite_error)?;
            return Ok(if existing.request_digest != request_digest {
                AuthoringInferenceBegin::Conflict(Box::new(existing))
            } else if existing.state.is_terminal() {
                AuthoringInferenceBegin::Replayed(Box::new(existing))
            } else {
                AuthoringInferenceBegin::InProgress(Box::new(existing))
            });
        }
        let record = AuthoringInferenceOperationRecord {
            schema_version: AUTHORING_INFERENCE_OPERATION_SCHEMA_VERSION.to_owned(),
            operation_id: operation_id.to_owned(),
            draft_id: draft_id.to_owned(),
            client_mutation_id: client_mutation_id.to_owned(),
            request_digest: request_digest.to_owned(),
            state: AuthoringInferenceOperationState::Pending,
            cost: reserved,
            proposal_id: None,
            proposal: None,
            detail: "reserved before provider exchange".to_owned(),
        };
        // A concurrent reservation of the same identity loses the insert and
        // reads the winner's row instead, so two callers cannot both start. The
        // losing caller must then be classified from that row rather than
        // assumed to have won: the row already existed before this insert, so it
        // is not the reservation this call just built, and reporting it as
        // `Started` would send a second caller to the provider for one identity
        // — the re-contact that the reservation exists to prevent. The rowcount
        // is what separates the winner's fresh row from the loser's read of a
        // row that may already be terminal.
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO management_authoring_inference_operations
                 (operation_id, draft_id, client_mutation_id, request_digest, record)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    operation_id,
                    draft_id,
                    client_mutation_id,
                    request_digest,
                    encode(&record)?
                ],
            )
            .map_err(sqlite_error)?;
        let stored = read_record(&transaction, operation_id)?.ok_or_else(|| {
            StoreError::new(
                "authoring_inference_operation_not_found",
                "the authoring-inference reservation did not persist",
            )
        })?;
        if inserted == 1 {
            debug_assert_eq!(
                stored.state,
                AuthoringInferenceOperationState::Pending,
                "a fresh reservation must persist the pending state it built"
            );
            transaction.commit().map_err(sqlite_error)?;
            return Ok(AuthoringInferenceBegin::Started(Box::new(stored)));
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(if stored.request_digest != request_digest {
            AuthoringInferenceBegin::Conflict(Box::new(stored))
        } else if stored.state.is_terminal() {
            AuthoringInferenceBegin::Replayed(Box::new(stored))
        } else {
            AuthoringInferenceBegin::InProgress(Box::new(stored))
        })
    }

    fn complete(
        &self,
        operation_id: &str,
        state: AuthoringInferenceOperationState,
        cost: AuthoringInferenceCost,
        proposal: Option<Box<AuthoringInferenceProposal>>,
        detail: &str,
    ) -> Result<AuthoringInferenceOperationRecord, StoreError> {
        if state == AuthoringInferenceOperationState::Pending {
            return Err(StoreError::new(
                "authoring_inference_journal_invalid_transition",
                "a completed operation cannot be reset to pending",
            ));
        }
        let mut connection = self.store.connection.lock().map_err(|_| poisoned())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let Some(current) = read_record(&transaction, operation_id)? else {
            transaction.commit().map_err(sqlite_error)?;
            return Err(StoreError::new(
                "authoring_inference_operation_not_found",
                "the authoring-inference operation was not reserved",
            ));
        };
        if current.state.is_terminal() && current.state != state {
            transaction.commit().map_err(sqlite_error)?;
            return Err(StoreError::new(
                "authoring_inference_journal_invalid_transition",
                "a terminal authoring-inference outcome cannot be rewritten",
            ));
        }
        // Every carried-over field is named rather than spread with `..current`,
        // so the idempotent-write check below can still read the state it
        // decided on instead of a partially moved record.
        let updated = AuthoringInferenceOperationRecord {
            schema_version: current.schema_version.clone(),
            operation_id: current.operation_id.clone(),
            draft_id: current.draft_id.clone(),
            client_mutation_id: current.client_mutation_id.clone(),
            request_digest: current.request_digest.clone(),
            state,
            cost,
            proposal_id: proposal.as_ref().map(|value| value.proposal_id.clone()),
            proposal,
            detail: bounded_detail(detail),
        };
        // The WHERE clause repeats the in-process terminal check as a predicate,
        // so a racing terminal write cannot be overwritten by a later one. The
        // transaction is what keeps the two in agreement: outside one, a
        // competitor can terminalize the row between the read above and this
        // update, leaving `current` stale so the guard at the top cannot fire,
        // and this caller is then handed `Ok` for an outcome the journal
        // silently discarded.
        let changed = transaction
            .execute(
                "UPDATE management_authoring_inference_operations
                 SET record = ?2
                 WHERE operation_id = ?1
                   AND json_extract(record, '$.state') = 'pending'",
                params![operation_id, encode(&updated)?],
            )
            .map_err(sqlite_error)?;
        if changed == 0 && current.state.is_terminal() && current.state == state {
            // An idempotent repeat of the winning terminal write: nothing changed,
            // but the lock is held until the transaction is closed.
            transaction.commit().map_err(sqlite_error)?;
            return Ok(current);
        }
        let stored = read_record(&transaction, operation_id)?.ok_or_else(|| {
            StoreError::new(
                "authoring_inference_operation_not_found",
                "the authoring-inference outcome did not persist",
            )
        })?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(stored)
    }

    fn get(
        &self,
        operation_id: &str,
    ) -> Result<Option<AuthoringInferenceOperationRecord>, StoreError> {
        let connection = self.store.connection.lock().map_err(|_| poisoned())?;
        read_record(&connection, operation_id)
    }
}

fn read_record(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Option<AuthoringInferenceOperationRecord>, StoreError> {
    let encoded = connection
        .query_row(
            "SELECT record FROM management_authoring_inference_operations WHERE operation_id = ?1",
            [operation_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    serde_json::from_str(&encoded).map(Some).map_err(|_| {
        StoreError::new(
            "authoring_inference_journal_corrupt",
            "the recorded authoring-inference operation is unreadable",
        )
    })
}

fn encode(record: &AuthoringInferenceOperationRecord) -> Result<String, StoreError> {
    serde_json::to_string(record).map_err(|_| {
        StoreError::new(
            "authoring_inference_journal_encode",
            "the authoring-inference operation could not be encoded",
        )
    })
}

fn poisoned() -> StoreError {
    StoreError::new(
        "authoring_inference_journal_poisoned",
        "authoring-inference journal lock is poisoned",
    )
}

fn sqlite_error(error: rusqlite::Error) -> StoreError {
    StoreError::new("authoring_inference_journal_sqlite", error.to_string())
}
