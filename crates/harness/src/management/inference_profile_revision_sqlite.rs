// SPDX-License-Identifier: MIT

//! Durable CAS journal for accepted inference-profile revisions.
//!
//! Each append is one `IMMEDIATE` transaction: it reads the accepted head,
//! compares it to the caller's expected digest, and inserts the new revision,
//! the head pointer and the mutation record together. Two concurrent edits
//! therefore cannot both win — the loser reads the winner's head and reports a
//! conflict instead of overwriting it.

use std::sync::Arc;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::super::contract::{InferenceProfileDescriptor, validate_identifier};
use super::super::store::{SqliteWorkflowStore, StoreError};
use super::{InferenceProfileRevisionJournal, RevisionAppendOutcome, validate_baseline};

/// Durable journal over the shared management SQLite store.
///
/// The store is held behind an `Arc` because the served composition passes the
/// same database to the run store and to this journal; both must observe one
/// connection so the transaction boundary is the journal's, not the caller's.
pub struct SqliteInferenceProfileRevisionJournal {
    store: Arc<SqliteWorkflowStore>,
}

impl SqliteInferenceProfileRevisionJournal {
    pub fn new(store: Arc<SqliteWorkflowStore>) -> Self {
        Self { store }
    }
}

impl InferenceProfileRevisionJournal for SqliteInferenceProfileRevisionJournal {
    fn head(&self, profile_id: &str) -> Result<Option<InferenceProfileDescriptor>, StoreError> {
        validate_identifier("profile_id", profile_id).map_err(request_error)?;
        let connection = self.store.connection.lock().map_err(|_| poisoned())?;
        read_head(&connection, profile_id)
    }

    fn history(&self, profile_id: &str) -> Result<Vec<InferenceProfileDescriptor>, StoreError> {
        validate_identifier("profile_id", profile_id).map_err(request_error)?;
        let connection = self.store.connection.lock().map_err(|_| poisoned())?;
        let mut statement = connection
            .prepare(
                "SELECT revision FROM management_inference_profile_revisions
                 WHERE profile_id = ?1 ORDER BY ordinal",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([profile_id], decode_revision_row)
            .map_err(sqlite_error)?;
        rows.map(|row| row.map_err(sqlite_error)).collect()
    }

    fn append(
        &self,
        profile_id: &str,
        baseline: &InferenceProfileDescriptor,
        expected_revision_digest: &str,
        client_mutation_id: &str,
        candidate: &InferenceProfileDescriptor,
    ) -> Result<RevisionAppendOutcome, StoreError> {
        validate_identifier("profile_id", profile_id).map_err(request_error)?;
        validate_identifier("client_mutation_id", client_mutation_id).map_err(request_error)?;
        validate_baseline(profile_id, baseline, candidate)?;
        let mut connection = self.store.connection.lock().map_err(|_| poisoned())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let current = match read_head(&transaction, profile_id)? {
            Some(current) => current,
            // First edit of a freshly opened journal: the revision the owner
            // currently serves becomes the first accepted revision, so the swap
            // below compares against what the owner actually publishes rather
            // than against nothing.
            None => {
                seed_baseline(&transaction, baseline)?;
                baseline.clone()
            }
        };
        let recorded = transaction
            .query_row(
                "SELECT mutation_digest, revision FROM management_inference_profile_mutations
                 WHERE profile_id = ?1 AND mutation_id = ?2",
                params![profile_id, client_mutation_id],
                |row| {
                    let digest: String = row.get(0)?;
                    let revision: Vec<u8> = row.get(1)?;
                    Ok((digest, revision))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        if let Some((digest, revision)) = recorded {
            transaction.commit().map_err(sqlite_error)?;
            return if digest == candidate.digest {
                Ok(RevisionAppendOutcome::Replayed(Box::new(decode_json(
                    &revision,
                )?)))
            } else {
                Ok(RevisionAppendOutcome::Conflict(Box::new(current)))
            };
        }
        if current.digest != expected_revision_digest {
            transaction.commit().map_err(sqlite_error)?;
            return Ok(RevisionAppendOutcome::Conflict(Box::new(current)));
        }
        let ordinal: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM management_inference_profile_revisions
                 WHERE profile_id = ?1",
                [profile_id],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        insert_revision(&transaction, profile_id, ordinal, candidate)?;
        transaction
            .execute(
                "INSERT INTO management_inference_profile_heads (profile_id, digest)
                 VALUES (?1, ?2)
                 ON CONFLICT(profile_id) DO UPDATE SET digest = excluded.digest",
                params![profile_id, candidate.digest],
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "INSERT INTO management_inference_profile_mutations
                 (profile_id, mutation_id, mutation_digest, revision)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    profile_id,
                    client_mutation_id,
                    candidate.digest,
                    encode_json(candidate)?
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(RevisionAppendOutcome::Adopted(Box::new(candidate.clone())))
    }
}

/// Records the revision the owner currently serves as the first accepted one.
fn seed_baseline(
    transaction: &rusqlite::Transaction<'_>,
    baseline: &InferenceProfileDescriptor,
) -> Result<(), StoreError> {
    insert_revision(transaction, &baseline.profile_id, 1, baseline)?;
    transaction
        .execute(
            "INSERT INTO management_inference_profile_heads (profile_id, digest) VALUES (?1, ?2)",
            params![baseline.profile_id, baseline.digest],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

fn insert_revision(
    transaction: &rusqlite::Transaction<'_>,
    profile_id: &str,
    ordinal: i64,
    revision: &InferenceProfileDescriptor,
) -> Result<(), StoreError> {
    transaction
        .execute(
            "INSERT INTO management_inference_profile_revisions
             (profile_id, ordinal, version, digest, revision)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                profile_id,
                ordinal,
                revision.version,
                revision.digest,
                encode_json(revision)?
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

fn read_head(
    connection: &rusqlite::Connection,
    profile_id: &str,
) -> Result<Option<InferenceProfileDescriptor>, StoreError> {
    connection
        .query_row(
            "SELECT revision FROM management_inference_profile_revisions
             WHERE profile_id = ?1 AND digest = (
                 SELECT digest FROM management_inference_profile_heads WHERE profile_id = ?1
             )",
            [profile_id],
            decode_revision_row,
        )
        .optional()
        .map_err(sqlite_error)
}

fn decode_revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InferenceProfileDescriptor> {
    let revision: Vec<u8> = row.get(0)?;
    decode_json(&revision).map_err(to_sql_error)
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value)
        .map_err(|error| StoreError::new("inference_profile_revision_encode", error.to_string()))
}

fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes)
        .map_err(|error| StoreError::new("inference_profile_revision_decode", error.to_string()))
}

fn sqlite_error(error: rusqlite::Error) -> StoreError {
    StoreError::new("inference_profile_revision_sqlite", error.to_string())
}

fn poisoned() -> StoreError {
    StoreError::new(
        "inference_profile_journal_poisoned",
        "inference-profile revision journal lock is poisoned",
    )
}

fn request_error(error: impl std::fmt::Display) -> StoreError {
    StoreError::new("inference_profile_revision_request", error.to_string())
}

fn to_sql_error(error: StoreError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}
