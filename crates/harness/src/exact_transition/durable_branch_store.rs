// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::validation::to_i64;
use super::{
    BranchArtifactReference, BranchStoreError, DURABLE_BRANCH_SCHEMA_VERSION, DurableBranch,
    DurableBranchDraft, DurableBranchStatus,
};
use crate::sha256_hex;

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// SQLite-backed owner store for durable branch metadata.
pub struct SqliteBranchStore {
    pub(crate) connection: Mutex<Connection>,
}

impl SqliteBranchStore {
    /// Opens or migrates a durable branch database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BranchStoreError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(BranchStoreError::InvalidInput);
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(BranchStoreError::Persistence(
                "branch store parent directory does not exist".to_owned(),
            ));
        }
        if path.is_file()
            && fs::metadata(path)
                .map_err(BranchStoreError::persistence)?
                .len()
                > 64 * 1024 * 1024
        {
            return Err(BranchStoreError::Capacity);
        }
        let mut connection = Connection::open(path).map_err(BranchStoreError::persistence)?;
        configure(&connection)?;
        super::store_migration::migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Opens an isolated in-memory branch database.
    pub fn open_in_memory() -> Result<Self, BranchStoreError> {
        let mut connection = Connection::open_in_memory().map_err(BranchStoreError::persistence)?;
        configure(&connection)?;
        super::store_migration::migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Returns the versioned contract persisted by this store.
    #[must_use]
    pub const fn schema_version() -> &'static str {
        DURABLE_BRANCH_SCHEMA_VERSION
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Connection>, BranchStoreError> {
        self.connection
            .lock()
            .map_err(|_| BranchStoreError::Persistence("branch store lock is poisoned".to_owned()))
    }
}

fn configure(connection: &Connection) -> Result<(), BranchStoreError> {
    connection
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(BranchStoreError::persistence)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(BranchStoreError::persistence)
}

pub(super) fn insert_artifacts(
    transaction: &Transaction<'_>,
    experiment_id: &str,
    branch_id: &str,
    operation_id: &str,
    references: &[BranchArtifactReference],
) -> Result<(), BranchStoreError> {
    for reference in references {
        transaction
            .execute(
                "INSERT INTO branch_artifacts(
                    experiment_id, branch_id, artifact_id, role, tombstoned, operation_id
                 ) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![
                    experiment_id,
                    branch_id,
                    reference.artifact_id,
                    reference.role.as_str(),
                    operation_id
                ],
            )
            .map_err(map_insert_error)?;
    }
    Ok(())
}

pub(super) fn strategy_label(strategy: super::BranchStrategy) -> &'static str {
    match strategy {
        super::BranchStrategy::ExactRestore => "exact_restore",
        super::BranchStrategy::PrefixReplay => "prefix_replay",
    }
}

pub(super) fn strategy_parse(value: &str) -> Result<super::BranchStrategy, BranchStoreError> {
    match value {
        "exact_restore" => Ok(super::BranchStrategy::ExactRestore),
        "prefix_replay" => Ok(super::BranchStrategy::PrefixReplay),
        _ => Err(BranchStoreError::Corrupt),
    }
}

pub(super) fn digest_create(draft: &DurableBranchDraft) -> String {
    let mut fields = vec![
        "create".to_owned(),
        draft.experiment_id.clone(),
        draft.root_branch_id.clone(),
        draft.branch_id.clone(),
        draft.parent_branch_id.clone().unwrap_or_default(),
        draft.fork.occurrence_id.as_str().to_owned(),
        draft
            .fork
            .parent_occurrence_id
            .as_ref()
            .map_or_else(String::new, |value| value.as_str().to_owned()),
        draft.fork.state_digest.as_str().to_owned(),
        strategy_label(draft.strategy).to_owned(),
        draft.source_handle.clone().unwrap_or_default(),
        draft.trajectory_prefix.clone().unwrap_or_default(),
        draft.effective_seed.clone().unwrap_or_default(),
        draft.setup_digest.clone().unwrap_or_default(),
        draft.boundary.clone(),
        draft.run_id.clone(),
        draft.episode_id.clone().unwrap_or_default(),
        draft.trajectory_id.clone().unwrap_or_default(),
        draft.context_id.clone().unwrap_or_default(),
        draft.policy_revision.clone(),
        draft.config_revision.clone(),
        draft.name.clone(),
        draft.notes.clone().unwrap_or_default(),
    ];
    let mut artifacts = draft.artifacts.clone();
    artifacts.sort();
    for artifact in artifacts {
        fields.push(artifact.artifact_id);
        fields.push(artifact.role.as_str().to_owned());
    }
    digest_fields(fields)
}

pub(super) fn digest_fields<I, S>(fields: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(field.as_ref().as_bytes());
        bytes.push(0);
    }
    format!("sha256:{}", sha256_hex(bytes))
}

pub(super) fn existing_operation(
    transaction: &Transaction<'_>,
    operation_id: &str,
    digest: &str,
) -> Result<Option<DurableBranch>, BranchStoreError> {
    let existing: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT payload_digest, experiment_id, branch_id FROM branch_operations
             WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(BranchStoreError::persistence)?;
    let Some((stored, experiment_id, branch_id)) = existing else {
        return Ok(None);
    };
    if stored != digest {
        return Err(BranchStoreError::IdempotencyConflict);
    }
    super::store_reads::load_branch_tx(transaction, &experiment_id, &branch_id)?
        .map(Some)
        .ok_or(BranchStoreError::Corrupt)
}

pub(super) fn record_operation(
    transaction: &Transaction<'_>,
    operation_id: &str,
    kind: &str,
    digest: &str,
    experiment_id: &str,
    branch_id: &str,
    now: i64,
) -> Result<(), BranchStoreError> {
    transaction
        .execute(
            "INSERT INTO branch_operations(
                operation_id, operation_kind, payload_digest, experiment_id, branch_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![operation_id, kind, digest, experiment_id, branch_id, now],
        )
        .map_err(map_insert_error)?;
    Ok(())
}

pub(super) struct EventInput<'a> {
    pub(super) experiment_id: &'a str,
    pub(super) branch_id: &'a str,
    pub(super) operation_id: &'a str,
    pub(super) kind: &'a str,
    pub(super) status: DurableBranchStatus,
    pub(super) metadata_revision: u64,
    pub(super) now: i64,
}

pub(super) fn append_event(
    transaction: &Transaction<'_>,
    input: EventInput<'_>,
) -> Result<(), BranchStoreError> {
    transaction
        .execute(
            "INSERT INTO branch_events(
                experiment_id, branch_id, operation_id, kind, status,
                metadata_revision, occurred_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                input.experiment_id,
                input.branch_id,
                input.operation_id,
                input.kind,
                input.status.as_str(),
                to_i64(input.metadata_revision)?,
                input.now
            ],
        )
        .map_err(BranchStoreError::persistence)?;
    Ok(())
}

pub(super) fn now_millis() -> Result<i64, BranchStoreError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(BranchStoreError::persistence)?;
    i64::try_from(duration.as_millis()).map_err(|_| BranchStoreError::InvalidInput)
}

pub(super) fn map_insert_error(error: rusqlite::Error) -> BranchStoreError {
    match error {
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            BranchStoreError::Duplicate
        }
        other => BranchStoreError::persistence(other),
    }
}

pub(super) fn begin_write_transaction(
    connection: &mut Connection,
) -> Result<Transaction<'_>, BranchStoreError> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(BranchStoreError::persistence)
}
