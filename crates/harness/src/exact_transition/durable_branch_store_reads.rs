// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::store::SqliteBranchStore;
pub(super) use super::store_bulk_reads::{all_branches, all_branches_tx};
use super::store_read_helpers::{
    BRANCH_COLUMNS, build_branch, load_artifacts, load_artifacts_tx, load_fork, load_fork_tx,
    read_raw_branch,
};
use super::validation::validate_label;
use super::{
    BranchEvent, BranchStoreError, DurableBranch, DurableBranchStatus, MAX_BRANCH_EVENT_PAGE,
    MAX_BRANCH_PAGE, MAX_TRANSITION_LABEL_BYTES,
};

/// A stable page of branch records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchPage {
    /// Records after the requested cursor.
    pub branches: Vec<DurableBranch>,
    /// Last branch ID to pass as the next cursor, when more records remain.
    pub next_cursor: Option<String>,
}

/// A durable page of branch events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchEventPage {
    /// Events after the requested sequence.
    pub events: Vec<BranchEvent>,
    /// Sequence to use for the next read.
    pub next_after_sequence: u64,
    /// Oldest event currently retained.
    pub oldest_sequence: Option<u64>,
    /// Newest event currently retained.
    pub newest_sequence: Option<u64>,
}

impl SqliteBranchStore {
    /// Looks up a branch by experiment and immutable branch identity.
    pub fn get(
        &self,
        experiment_id: &str,
        branch_id: &str,
    ) -> Result<Option<DurableBranch>, BranchStoreError> {
        validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(branch_id, MAX_TRANSITION_LABEL_BYTES)?;
        let connection = self.lock()?;
        load_branch(&connection, experiment_id, branch_id)
    }

    /// Lists branches in stable branch-ID order using an opaque cursor.
    pub fn list(
        &self,
        experiment_id: &str,
        cursor: Option<&str>,
        limit: u64,
    ) -> Result<BranchPage, BranchStoreError> {
        validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
        if let Some(cursor) = cursor {
            validate_label(cursor, MAX_TRANSITION_LABEL_BYTES)?;
        }
        let bounded = limit.clamp(1, MAX_BRANCH_PAGE);
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {BRANCH_COLUMNS} FROM durable_branches
                 WHERE experiment_id = ?1 AND branch_id > ?2
                 ORDER BY branch_id LIMIT ?3"
            ))
            .map_err(BranchStoreError::persistence)?;
        let rows = statement
            .query_map(
                params![
                    experiment_id,
                    cursor.unwrap_or_default(),
                    i64::try_from(bounded + 1).map_err(|_| BranchStoreError::InvalidInput)?
                ],
                read_raw_branch,
            )
            .map_err(BranchStoreError::persistence)?;
        let raw = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(BranchStoreError::persistence)?;
        let mut branches = Vec::with_capacity(raw.len());
        for value in raw {
            let artifacts = load_artifacts(&connection, experiment_id, &value.branch_id)?;
            let fork = load_fork(&connection, experiment_id, &value.fork_occurrence_id)?;
            branches.push(build_branch(value, artifacts, fork)?);
        }
        let next_cursor = if branches.len() > bounded as usize {
            branches.pop();
            branches.last().map(|branch| branch.branch_id.clone())
        } else {
            None
        };
        Ok(BranchPage {
            branches,
            next_cursor,
        })
    }

    /// Returns an immutable ancestry chain, root first.
    pub fn ancestry(
        &self,
        experiment_id: &str,
        branch_id: &str,
    ) -> Result<Vec<DurableBranch>, BranchStoreError> {
        validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(branch_id, MAX_TRANSITION_LABEL_BYTES)?;
        let connection = self.lock()?;
        let mut result = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = Some(branch_id.to_owned());
        while let Some(id) = current {
            if !seen.insert(id.clone()) {
                return Err(BranchStoreError::Corrupt);
            }
            let branch = load_branch(&connection, experiment_id, &id)?
                .ok_or(BranchStoreError::UnknownBranch)?;
            current = branch.parent_branch_id.clone();
            result.push(branch);
            if result.len() > super::MAX_BRANCHES {
                return Err(BranchStoreError::Corrupt);
            }
        }
        result.reverse();
        Ok(result)
    }

    /// Reads append-only branch events after a durable sequence cursor.
    pub fn events(
        &self,
        experiment_id: &str,
        after_sequence: u64,
        limit: u64,
    ) -> Result<BranchEventPage, BranchStoreError> {
        validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
        let bounded = limit.clamp(1, MAX_BRANCH_EVENT_PAGE);
        let after = i64::try_from(after_sequence).map_err(|_| BranchStoreError::InvalidCursor)?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT event_id, experiment_id, branch_id, operation_id, kind, status,
                        metadata_revision, occurred_at
                 FROM branch_events
                 WHERE experiment_id = ?1 AND event_id > ?2
                 ORDER BY event_id LIMIT ?3",
            )
            .map_err(BranchStoreError::persistence)?;
        let rows = statement
            .query_map(
                params![
                    experiment_id,
                    after,
                    i64::try_from(bounded + 1).map_err(|_| BranchStoreError::InvalidInput)?
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .map_err(BranchStoreError::persistence)?;
        let mut events = Vec::new();
        for row in rows {
            let (
                sequence,
                experiment_id,
                branch_id,
                operation_id,
                kind,
                status,
                metadata_revision,
                occurred_at,
            ) = row.map_err(BranchStoreError::persistence)?;
            events.push(BranchEvent {
                sequence: u64::try_from(sequence).map_err(|_| BranchStoreError::Corrupt)?,
                experiment_id,
                branch_id,
                operation_id,
                kind,
                status: DurableBranchStatus::parse(&status)?,
                metadata_revision: u64::try_from(metadata_revision)
                    .map_err(|_| BranchStoreError::Corrupt)?,
                occurred_at,
            });
        }
        if events.len() > bounded as usize {
            events.pop();
        }
        let next_after_sequence = events.last().map_or(after_sequence, |event| event.sequence);
        let oldest_sequence = connection
            .query_row(
                "SELECT MIN(event_id) FROM branch_events WHERE experiment_id = ?1",
                [experiment_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(BranchStoreError::persistence)?
            .map(|value| u64::try_from(value).map_err(|_| BranchStoreError::Corrupt))
            .transpose()?;
        let newest_sequence = connection
            .query_row(
                "SELECT MAX(event_id) FROM branch_events WHERE experiment_id = ?1",
                [experiment_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(BranchStoreError::persistence)?
            .map(|value| u64::try_from(value).map_err(|_| BranchStoreError::Corrupt))
            .transpose()?;
        Ok(BranchEventPage {
            events,
            next_after_sequence,
            oldest_sequence,
            newest_sequence,
        })
    }
}

pub(super) fn load_branch_tx(
    transaction: &Transaction<'_>,
    experiment_id: &str,
    branch_id: &str,
) -> Result<Option<DurableBranch>, BranchStoreError> {
    let raw = transaction
        .query_row(
            &format!(
                "SELECT {BRANCH_COLUMNS} FROM durable_branches
                      WHERE experiment_id = ?1 AND branch_id = ?2"
            ),
            params![experiment_id, branch_id],
            read_raw_branch,
        )
        .optional()
        .map_err(BranchStoreError::persistence)?;
    raw.map(|value| {
        let artifacts = load_artifacts_tx(transaction, experiment_id, branch_id)?;
        let fork = load_fork_tx(transaction, experiment_id, &value.fork_occurrence_id)?;
        build_branch(value, artifacts, fork)
    })
    .transpose()
}

fn load_branch(
    connection: &Connection,
    experiment_id: &str,
    branch_id: &str,
) -> Result<Option<DurableBranch>, BranchStoreError> {
    let raw = connection
        .query_row(
            &format!(
                "SELECT {BRANCH_COLUMNS} FROM durable_branches
                      WHERE experiment_id = ?1 AND branch_id = ?2"
            ),
            params![experiment_id, branch_id],
            read_raw_branch,
        )
        .optional()
        .map_err(BranchStoreError::persistence)?;
    raw.map(|value| {
        let artifacts = load_artifacts(connection, experiment_id, branch_id)?;
        let fork = load_fork(connection, experiment_id, &value.fork_occurrence_id)?;
        build_branch(value, artifacts, fork)
    })
    .transpose()
}
