// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, Transaction, params};

use super::store_retention::BranchPrunePlan;
use super::validation::validate_label;
use super::{
    BranchArtifactReference, BranchArtifactRole, BranchStoreError, MAX_TRANSITION_LABEL_BYTES,
};

pub(super) fn insert_prune_plan(
    transaction: &Transaction<'_>,
    operation_id: &str,
    experiment_id: &str,
    plan: &BranchPrunePlan,
) -> Result<(), BranchStoreError> {
    transaction
        .execute(
            "INSERT INTO branch_prune_plans(
                operation_id, experiment_id, branch_ids, retained_artifacts, collectable_artifacts
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                operation_id,
                experiment_id,
                encode_labels(&plan.branch_ids),
                encode_references(&plan.retained_artifacts),
                encode_references(&plan.collectable_artifacts)
            ],
        )
        .map_err(BranchStoreError::persistence)?;
    Ok(())
}

pub(super) fn load_prune_plan(
    transaction: &Transaction<'_>,
    operation_id: &str,
) -> Result<Option<BranchPrunePlan>, BranchStoreError> {
    let encoded: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT branch_ids, retained_artifacts, collectable_artifacts
             FROM branch_prune_plans WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(BranchStoreError::persistence)?;
    encoded
        .map(|(branch_ids, retained_artifacts, collectable_artifacts)| {
            Ok(BranchPrunePlan {
                branch_ids: decode_labels(&branch_ids)?,
                retained_artifacts: decode_references(&retained_artifacts)?,
                collectable_artifacts: decode_references(&collectable_artifacts)?,
            })
        })
        .transpose()
}

fn encode_labels(labels: &[String]) -> String {
    labels.join("\n")
}

fn encode_references(references: &[BranchArtifactReference]) -> String {
    references
        .iter()
        .map(|reference| format!("{}|{}", reference.artifact_id, reference.role.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_labels(encoded: &str) -> Result<Vec<String>, BranchStoreError> {
    if encoded.is_empty() {
        return Ok(Vec::new());
    }
    encoded
        .split('\n')
        .map(|label| {
            validate_label(label, MAX_TRANSITION_LABEL_BYTES)?;
            Ok(label.to_owned())
        })
        .collect()
}

fn decode_references(encoded: &str) -> Result<Vec<BranchArtifactReference>, BranchStoreError> {
    if encoded.is_empty() {
        return Ok(Vec::new());
    }
    encoded
        .split('\n')
        .map(|value| {
            let (artifact_id, role) = value.split_once('|').ok_or(BranchStoreError::Corrupt)?;
            validate_label(artifact_id, MAX_TRANSITION_LABEL_BYTES)?;
            Ok(BranchArtifactReference {
                artifact_id: artifact_id.to_owned(),
                role: BranchArtifactRole::parse(role)?,
            })
        })
        .collect()
}
