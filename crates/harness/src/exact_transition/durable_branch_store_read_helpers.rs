// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Row, Transaction, params};

use super::store::strategy_parse;
use super::{
    BranchArtifactReference, BranchArtifactRole, BranchAssurance, BranchFork, BranchStoreError,
    DurableBranch, DurableBranchStatus, OccurrenceId,
};
use crate::execution::ExactStateDigest;

pub(super) const BRANCH_COLUMNS: &str =
    "experiment_id, root_branch_id, branch_id, parent_branch_id,
    fork_occurrence_id, strategy, source_handle, trajectory_prefix, effective_seed, setup_digest,
    boundary, assurance, run_id, episode_id, trajectory_id, context_id, policy_revision,
    config_revision, name, notes, status, metadata_revision, created_at, updated_at";

#[derive(Clone, Debug)]
pub(super) struct RawBranch {
    pub(super) experiment_id: String,
    pub(super) root_branch_id: String,
    pub(super) branch_id: String,
    pub(super) parent_branch_id: Option<String>,
    pub(super) fork_occurrence_id: String,
    pub(super) strategy: String,
    pub(super) source_handle: Option<String>,
    pub(super) trajectory_prefix: Option<String>,
    pub(super) effective_seed: Option<String>,
    pub(super) setup_digest: Option<String>,
    pub(super) boundary: String,
    pub(super) assurance: String,
    pub(super) run_id: String,
    pub(super) episode_id: Option<String>,
    pub(super) trajectory_id: Option<String>,
    pub(super) context_id: Option<String>,
    pub(super) policy_revision: String,
    pub(super) config_revision: String,
    pub(super) name: String,
    pub(super) notes: Option<String>,
    pub(super) status: String,
    pub(super) metadata_revision: i64,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

pub(super) fn read_raw_branch(row: &Row<'_>) -> rusqlite::Result<RawBranch> {
    Ok(RawBranch {
        experiment_id: row.get(0)?,
        root_branch_id: row.get(1)?,
        branch_id: row.get(2)?,
        parent_branch_id: row.get(3)?,
        fork_occurrence_id: row.get(4)?,
        strategy: row.get(5)?,
        source_handle: row.get(6)?,
        trajectory_prefix: row.get(7)?,
        effective_seed: row.get(8)?,
        setup_digest: row.get(9)?,
        boundary: row.get(10)?,
        assurance: row.get(11)?,
        run_id: row.get(12)?,
        episode_id: row.get(13)?,
        trajectory_id: row.get(14)?,
        context_id: row.get(15)?,
        policy_revision: row.get(16)?,
        config_revision: row.get(17)?,
        name: row.get(18)?,
        notes: row.get(19)?,
        status: row.get(20)?,
        metadata_revision: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

pub(super) fn build_branch(
    raw: RawBranch,
    artifacts: Vec<BranchArtifactReference>,
    fork: (Option<String>, String),
) -> Result<DurableBranch, BranchStoreError> {
    let (parent_occurrence_id, state_digest) = fork;
    Ok(DurableBranch {
        experiment_id: raw.experiment_id,
        root_branch_id: raw.root_branch_id,
        branch_id: raw.branch_id,
        parent_branch_id: raw.parent_branch_id,
        fork: BranchFork {
            occurrence_id: OccurrenceId::parse(&raw.fork_occurrence_id)
                .map_err(|_| BranchStoreError::Corrupt)?,
            parent_occurrence_id: parent_occurrence_id
                .map(|value| OccurrenceId::parse(&value))
                .transpose()
                .map_err(|_| BranchStoreError::Corrupt)?,
            state_digest: ExactStateDigest::parse(&state_digest)
                .map_err(|_| BranchStoreError::Corrupt)?,
        },
        strategy: strategy_parse(&raw.strategy)?,
        source_handle: raw.source_handle,
        trajectory_prefix: raw.trajectory_prefix,
        effective_seed: raw.effective_seed,
        setup_digest: raw.setup_digest,
        boundary: raw.boundary,
        assurance: BranchAssurance::parse(&raw.assurance)?,
        run_id: raw.run_id,
        episode_id: raw.episode_id,
        trajectory_id: raw.trajectory_id,
        context_id: raw.context_id,
        policy_revision: raw.policy_revision,
        config_revision: raw.config_revision,
        name: raw.name,
        notes: raw.notes,
        status: DurableBranchStatus::parse(&raw.status)?,
        metadata_revision: u64::try_from(raw.metadata_revision)
            .map_err(|_| BranchStoreError::Corrupt)?,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
        artifacts,
    })
}

pub(super) fn load_artifacts(
    connection: &Connection,
    experiment_id: &str,
    branch_id: &str,
) -> Result<Vec<BranchArtifactReference>, BranchStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT artifact_id, role FROM branch_artifacts
             WHERE experiment_id = ?1 AND branch_id = ?2 AND tombstoned = 0
             ORDER BY artifact_id, role",
        )
        .map_err(BranchStoreError::persistence)?;
    let rows = statement
        .query_map(params![experiment_id, branch_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(BranchStoreError::persistence)?;
    rows.map(|row| {
        let (artifact_id, role) = row.map_err(BranchStoreError::persistence)?;
        Ok(BranchArtifactReference {
            artifact_id,
            role: BranchArtifactRole::parse(&role)?,
        })
    })
    .collect()
}

pub(super) fn load_fork(
    connection: &Connection,
    experiment_id: &str,
    occurrence_id: &str,
) -> Result<(Option<String>, String), BranchStoreError> {
    connection
        .query_row(
            "SELECT parent_occurrence_id, state_digest FROM branch_occurrences
             WHERE experiment_id = ?1 AND occurrence_id = ?2",
            params![experiment_id, occurrence_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => BranchStoreError::Corrupt,
            other => BranchStoreError::persistence(other),
        })
}

pub(super) fn load_artifacts_tx(
    transaction: &Transaction<'_>,
    experiment_id: &str,
    branch_id: &str,
) -> Result<Vec<BranchArtifactReference>, BranchStoreError> {
    let mut statement = transaction
        .prepare(
            "SELECT artifact_id, role FROM branch_artifacts
             WHERE experiment_id = ?1 AND branch_id = ?2 AND tombstoned = 0
             ORDER BY artifact_id, role",
        )
        .map_err(BranchStoreError::persistence)?;
    let rows = statement
        .query_map(params![experiment_id, branch_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(BranchStoreError::persistence)?;
    rows.map(|row| {
        let (artifact_id, role) = row.map_err(BranchStoreError::persistence)?;
        Ok(BranchArtifactReference {
            artifact_id,
            role: BranchArtifactRole::parse(&role)?,
        })
    })
    .collect()
}

pub(super) fn load_fork_tx(
    transaction: &Transaction<'_>,
    experiment_id: &str,
    occurrence_id: &str,
) -> Result<(Option<String>, String), BranchStoreError> {
    transaction
        .query_row(
            "SELECT parent_occurrence_id, state_digest FROM branch_occurrences
             WHERE experiment_id = ?1 AND occurrence_id = ?2",
            params![experiment_id, occurrence_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => BranchStoreError::Corrupt,
            other => BranchStoreError::persistence(other),
        })
}
