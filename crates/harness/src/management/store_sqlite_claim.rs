// SPDX-License-Identifier: MIT

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::super::super::contract::{
    MAX_SUBMISSION_CLAIMS, SubmissionClaim, SubmissionClaimAcceptance,
};
use super::super::submission_claim::{accepts, capacity, conflict, validate};
use super::support::{connection, decode, encode, sqlite_error};
use super::{SqliteWorkflowStore, StoreError};

fn lookup(
    connection: &Connection,
    request_id: &str,
) -> Result<Option<SubmissionClaim>, StoreError> {
    let record = connection
        .query_row(
            "SELECT record FROM management_submission_claims WHERE request_id = ?1",
            [request_id],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    record.map(|bytes| decode(&bytes)).transpose()
}

pub(super) fn read(
    store: &SqliteWorkflowStore,
    request_id: &str,
) -> Result<Option<SubmissionClaim>, StoreError> {
    let connection = connection(store)?;
    lookup(&connection, request_id)
}

pub(super) fn acquire(
    store: &SqliteWorkflowStore,
    request_id: &str,
    proposed: SubmissionClaim,
) -> Result<SubmissionClaimAcceptance, StoreError> {
    validate(request_id, &proposed)?;
    if proposed.outcome.is_some() {
        return Err(conflict());
    }
    let mut connection = connection(store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    if let Some(existing) = lookup(&transaction, request_id)? {
        return Ok(if accepts(&existing, &proposed) {
            SubmissionClaimAcceptance::Existing(existing)
        } else {
            SubmissionClaimAcceptance::Conflict
        });
    }
    let legacy: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM management_submissions WHERE request_id = ?1)",
            [request_id],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if legacy {
        return Ok(SubmissionClaimAcceptance::Conflict);
    }
    let count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM management_submission_claims",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if count >= MAX_SUBMISSION_CLAIMS as i64 {
        return Err(capacity());
    }
    transaction
        .execute(
            "INSERT INTO management_submission_claims(request_id, record) VALUES (?1, ?2)",
            params![request_id, encode(&proposed)?],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)?;
    Ok(SubmissionClaimAcceptance::Acquired)
}

pub(super) fn finish(
    store: &SqliteWorkflowStore,
    request_id: &str,
    proposed: SubmissionClaim,
) -> Result<(), StoreError> {
    validate(request_id, &proposed)?;
    let mut connection = connection(store)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let existing = lookup(&transaction, request_id)?.ok_or_else(conflict)?;
    if !accepts(&existing, &proposed)
        || proposed.outcome.is_none()
        || existing
            .outcome
            .as_ref()
            .is_some_and(|value| Some(value) != proposed.outcome.as_ref())
    {
        return Err(conflict());
    }
    transaction
        .execute(
            "UPDATE management_submission_claims SET record = ?2 WHERE request_id = ?1",
            params![request_id, encode(&proposed)?],
        )
        .map_err(sqlite_error)?;
    transaction.commit().map_err(sqlite_error)
}
