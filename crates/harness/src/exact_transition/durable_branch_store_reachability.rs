// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use rusqlite::{Connection, Transaction};

use super::{BranchArtifactReference, BranchArtifactRole, BranchStoreError};

pub(super) struct Reachability {
    pub(super) retained: Vec<BranchArtifactReference>,
    pub(super) collectable: Vec<BranchArtifactReference>,
}

struct GlobalArtifact {
    experiment_id: String,
    branch_id: String,
    reference: BranchArtifactReference,
}

pub(super) fn from_connection(
    connection: &Connection,
    experiment_id: &str,
    selected: &[String],
) -> Result<Reachability, BranchStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT a.experiment_id, a.branch_id, a.artifact_id, a.role
             FROM branch_artifacts a
             WHERE a.tombstoned = 0
               AND NOT EXISTS (
                   SELECT 1 FROM branch_tombstones t
                   WHERE t.experiment_id = a.experiment_id AND t.branch_id = a.branch_id
               )
             ORDER BY a.experiment_id, a.branch_id, a.artifact_id, a.role",
        )
        .map_err(BranchStoreError::persistence)?;
    let rows = statement
        .query_map([], read_global_artifact)
        .map_err(BranchStoreError::persistence)?;
    let artifacts = rows
        .map(|row| {
            row.map_err(BranchStoreError::persistence)
                .and_then(parse_global_artifact)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(classify(artifacts, experiment_id, selected))
}

pub(super) fn from_transaction(
    transaction: &Transaction<'_>,
    experiment_id: &str,
    selected: &[String],
) -> Result<Reachability, BranchStoreError> {
    let mut statement = transaction
        .prepare(
            "SELECT a.experiment_id, a.branch_id, a.artifact_id, a.role
             FROM branch_artifacts a
             WHERE a.tombstoned = 0
               AND NOT EXISTS (
                   SELECT 1 FROM branch_tombstones t
                   WHERE t.experiment_id = a.experiment_id AND t.branch_id = a.branch_id
               )
             ORDER BY a.experiment_id, a.branch_id, a.artifact_id, a.role",
        )
        .map_err(BranchStoreError::persistence)?;
    let rows = statement
        .query_map([], read_global_artifact)
        .map_err(BranchStoreError::persistence)?;
    let artifacts = rows
        .map(|row| {
            row.map_err(BranchStoreError::persistence)
                .and_then(parse_global_artifact)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(classify(artifacts, experiment_id, selected))
}

fn read_global_artifact(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn parse_global_artifact(
    row: (String, String, String, String),
) -> Result<GlobalArtifact, BranchStoreError> {
    let (experiment_id, branch_id, artifact_id, role) = row;
    Ok(GlobalArtifact {
        experiment_id,
        branch_id,
        reference: BranchArtifactReference {
            artifact_id,
            role: BranchArtifactRole::parse(&role)?,
        },
    })
}

fn classify(
    artifacts: Vec<GlobalArtifact>,
    experiment_id: &str,
    selected: &[String],
) -> Reachability {
    let selected_set: BTreeSet<&str> = selected.iter().map(String::as_str).collect();
    let mut outside_refs = BTreeSet::new();
    let mut outside_ids = BTreeSet::new();
    let mut inside = BTreeSet::new();
    for artifact in artifacts {
        if artifact.experiment_id == experiment_id
            && selected_set.contains(artifact.branch_id.as_str())
        {
            inside.insert(artifact.reference);
        } else {
            outside_ids.insert(artifact.reference.artifact_id.clone());
            outside_refs.insert(artifact.reference);
        }
    }
    let mut retained = outside_refs.into_iter().collect::<Vec<_>>();
    let mut collectable = Vec::new();
    for reference in inside {
        if outside_ids.contains(&reference.artifact_id) {
            retained.push(reference);
        } else {
            collectable.push(reference);
        }
    }
    retained.sort();
    retained.dedup();
    collectable.sort();
    collectable.dedup();
    Reachability {
        retained,
        collectable,
    }
}
