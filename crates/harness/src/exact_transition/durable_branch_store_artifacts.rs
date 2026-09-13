// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::store::{
    EventInput, SqliteBranchStore, existing_operation, now_millis, record_operation,
};
use super::validation::{to_i64, validate_label};
use super::{
    BranchArtifactReference, BranchStoreError, DurableBranch, MAX_BRANCH_ARTIFACTS,
    MAX_TRANSITION_LABEL_BYTES,
};

fn validate_operation_labels(
    operation_id: &str,
    experiment_id: &str,
    branch_id: &str,
) -> Result<(), BranchStoreError> {
    validate_label(operation_id, MAX_TRANSITION_LABEL_BYTES)?;
    validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
    validate_label(branch_id, MAX_TRANSITION_LABEL_BYTES)
}

impl SqliteBranchStore {
    /// Adds one artifact reachability edge with metadata CAS.
    pub fn attach_artifact(
        &self,
        operation_id: &str,
        experiment_id: &str,
        branch_id: &str,
        expected_revision: u64,
        reference: BranchArtifactReference,
    ) -> Result<DurableBranch, BranchStoreError> {
        validate_operation_labels(operation_id, experiment_id, branch_id)?;
        validate_label(&reference.artifact_id, MAX_TRANSITION_LABEL_BYTES)?;
        let digest = super::store::digest_fields([
            "artifact",
            experiment_id,
            branch_id,
            &expected_revision.to_string(),
            &reference.artifact_id,
            reference.role.as_str(),
        ]);
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(BranchStoreError::persistence)?;
        if let Some(existing) = existing_operation(&transaction, operation_id, &digest)? {
            transaction
                .commit()
                .map_err(BranchStoreError::persistence)?;
            drop(connection);
            return Ok(existing);
        }
        let branch = super::store_reads::load_branch_tx(&transaction, experiment_id, branch_id)?
            .ok_or(BranchStoreError::UnknownBranch)?;
        if branch.metadata_revision != expected_revision {
            return Err(BranchStoreError::StaleRevision);
        }
        if branch.artifacts.contains(&reference) {
            return Err(BranchStoreError::Duplicate);
        }
        if branch.artifacts.len() >= MAX_BRANCH_ARTIFACTS {
            return Err(BranchStoreError::Capacity);
        }
        let tombstoned: Option<i64> = transaction
            .query_row(
                "SELECT tombstoned FROM branch_artifacts
                 WHERE experiment_id = ?1 AND branch_id = ?2 AND artifact_id = ?3 AND role = ?4",
                params![
                    experiment_id,
                    branch_id,
                    reference.artifact_id,
                    reference.role.as_str()
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(BranchStoreError::persistence)?;
        if tombstoned.is_some() {
            return Err(BranchStoreError::ArtifactUnavailable);
        }
        let revision = branch
            .metadata_revision
            .checked_add(1)
            .ok_or(BranchStoreError::InvalidInput)?;
        let now = now_millis()?;
        transaction
            .execute(
                "UPDATE durable_branches SET metadata_revision = ?3, updated_at = ?4
                 WHERE experiment_id = ?1 AND branch_id = ?2",
                params![experiment_id, branch_id, to_i64(revision)?, now],
            )
            .map_err(BranchStoreError::persistence)?;
        record_operation(
            &transaction,
            operation_id,
            "artifact",
            &digest,
            experiment_id,
            branch_id,
            now,
        )?;
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
            .map_err(super::store::map_insert_error)?;
        super::store::append_event(
            &transaction,
            EventInput {
                experiment_id,
                branch_id,
                operation_id,
                kind: "artifact_attached",
                status: branch.status,
                metadata_revision: revision,
                now,
            },
        )?;
        transaction
            .commit()
            .map_err(BranchStoreError::persistence)?;
        drop(connection);
        self.get(experiment_id, branch_id)?
            .ok_or(BranchStoreError::Corrupt)
    }
}
