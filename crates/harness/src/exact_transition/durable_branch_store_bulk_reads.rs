// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Transaction};

use super::store_read_helpers::{
    BRANCH_COLUMNS, build_branch, load_artifacts, load_artifacts_tx, load_fork, load_fork_tx,
    read_raw_branch,
};
use super::{BranchStoreError, DurableBranch};

pub(super) fn all_branches(
    connection: &Connection,
    experiment_id: &str,
) -> Result<Vec<DurableBranch>, BranchStoreError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {BRANCH_COLUMNS} FROM durable_branches
             WHERE experiment_id = ?1 ORDER BY branch_id"
        ))
        .map_err(BranchStoreError::persistence)?;
    let rows = statement
        .query_map([experiment_id], read_raw_branch)
        .map_err(BranchStoreError::persistence)?;
    let raw = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(BranchStoreError::persistence)?;
    raw.into_iter()
        .map(|value| {
            let artifacts = load_artifacts(connection, experiment_id, &value.branch_id)?;
            let fork = load_fork(connection, experiment_id, &value.fork_occurrence_id)?;
            build_branch(value, artifacts, fork)
        })
        .collect()
}

pub(super) fn all_branches_tx(
    transaction: &Transaction<'_>,
    experiment_id: &str,
) -> Result<Vec<DurableBranch>, BranchStoreError> {
    let mut statement = transaction
        .prepare(&format!(
            "SELECT {BRANCH_COLUMNS} FROM durable_branches
             WHERE experiment_id = ?1 ORDER BY branch_id"
        ))
        .map_err(BranchStoreError::persistence)?;
    let rows = statement
        .query_map([experiment_id], read_raw_branch)
        .map_err(BranchStoreError::persistence)?;
    let raw = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(BranchStoreError::persistence)?;
    raw.into_iter()
        .map(|value| {
            let artifacts = load_artifacts_tx(transaction, experiment_id, &value.branch_id)?;
            let fork = load_fork_tx(transaction, experiment_id, &value.fork_occurrence_id)?;
            build_branch(value, artifacts, fork)
        })
        .collect()
}
