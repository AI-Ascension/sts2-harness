// SPDX-License-Identifier: MIT

use rusqlite::params;

use super::store::{
    EventInput, SqliteBranchStore, existing_operation, now_millis, record_operation,
};
use super::validation::{readiness_assured, to_i64, transition_allowed, validate_label};
use super::{
    BranchAssurance, BranchStoreError, DurableBranch, DurableBranchStatus,
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
    /// Applies a lifecycle transition with an expected metadata revision.
    pub fn transition(
        &self,
        operation_id: &str,
        experiment_id: &str,
        branch_id: &str,
        expected_revision: u64,
        status: DurableBranchStatus,
    ) -> Result<DurableBranch, BranchStoreError> {
        validate_operation_labels(operation_id, experiment_id, branch_id)?;
        let digest = super::store::digest_fields([
            "transition",
            experiment_id,
            branch_id,
            &expected_revision.to_string(),
            status.as_str(),
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
        if !transition_allowed(branch.status, status) {
            return Err(BranchStoreError::InvalidTransition);
        }
        if status == DurableBranchStatus::Ready
            && !readiness_assured(branch.strategy, branch.assurance)
        {
            return Err(BranchStoreError::InsufficientAssurance);
        }
        if status == DurableBranchStatus::Restoring
            && branch.strategy != super::BranchStrategy::ExactRestore
        {
            return Err(BranchStoreError::InvalidTransition);
        }
        if status == DurableBranchStatus::Replaying
            && branch.strategy != super::BranchStrategy::PrefixReplay
        {
            return Err(BranchStoreError::InvalidTransition);
        }
        let revision = branch
            .metadata_revision
            .checked_add(1)
            .ok_or(BranchStoreError::InvalidInput)?;
        let now = now_millis()?;
        transaction
            .execute(
                "UPDATE durable_branches
                 SET status = ?3, metadata_revision = ?4, updated_at = ?5
                 WHERE experiment_id = ?1 AND branch_id = ?2",
                params![
                    experiment_id,
                    branch_id,
                    status.as_str(),
                    to_i64(revision)?,
                    now
                ],
            )
            .map_err(BranchStoreError::persistence)?;
        record_operation(
            &transaction,
            operation_id,
            "transition",
            &digest,
            experiment_id,
            branch_id,
            now,
        )?;
        super::store::append_event(
            &transaction,
            EventInput {
                experiment_id,
                branch_id,
                operation_id,
                kind: "transitioned",
                status,
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

    /// Records verified strategy evidence without starting a continuation.
    pub fn set_assurance(
        &self,
        operation_id: &str,
        experiment_id: &str,
        branch_id: &str,
        expected_revision: u64,
        assurance: BranchAssurance,
    ) -> Result<DurableBranch, BranchStoreError> {
        validate_operation_labels(operation_id, experiment_id, branch_id)?;
        let digest = super::store::digest_fields([
            "assurance",
            experiment_id,
            branch_id,
            &expected_revision.to_string(),
            assurance.as_str(),
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
        if assurance != BranchAssurance::Unverified
            && !readiness_assured(branch.strategy, assurance)
        {
            return Err(BranchStoreError::InvalidInput);
        }
        if !matches!(
            branch.status,
            DurableBranchStatus::Restoring
                | DurableBranchStatus::Replaying
                | DurableBranchStatus::Unknown
        ) {
            return Err(BranchStoreError::InvalidTransition);
        }
        let revision = branch
            .metadata_revision
            .checked_add(1)
            .ok_or(BranchStoreError::InvalidInput)?;
        let now = now_millis()?;
        transaction
            .execute(
                "UPDATE durable_branches
                 SET assurance = ?3, metadata_revision = ?4, updated_at = ?5
                 WHERE experiment_id = ?1 AND branch_id = ?2",
                params![
                    experiment_id,
                    branch_id,
                    assurance.as_str(),
                    to_i64(revision)?,
                    now
                ],
            )
            .map_err(BranchStoreError::persistence)?;
        record_operation(
            &transaction,
            operation_id,
            "assurance",
            &digest,
            experiment_id,
            branch_id,
            now,
        )?;
        super::store::append_event(
            &transaction,
            EventInput {
                experiment_id,
                branch_id,
                operation_id,
                kind: "assurance_recorded",
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

    /// Renames a branch using a metadata CAS revision.
    pub fn rename(
        &self,
        operation_id: &str,
        experiment_id: &str,
        branch_id: &str,
        expected_revision: u64,
        name: &str,
    ) -> Result<DurableBranch, BranchStoreError> {
        validate_operation_labels(operation_id, experiment_id, branch_id)?;
        validate_label(name, super::MAX_BRANCH_NAME_BYTES)?;
        let digest = super::store::digest_fields([
            "rename",
            experiment_id,
            branch_id,
            &expected_revision.to_string(),
            name,
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
        let revision = branch
            .metadata_revision
            .checked_add(1)
            .ok_or(BranchStoreError::InvalidInput)?;
        let now = now_millis()?;
        transaction
            .execute(
                "UPDATE durable_branches SET name = ?3, metadata_revision = ?4, updated_at = ?5
                 WHERE experiment_id = ?1 AND branch_id = ?2",
                params![experiment_id, branch_id, name, to_i64(revision)?, now],
            )
            .map_err(BranchStoreError::persistence)?;
        record_operation(
            &transaction,
            operation_id,
            "rename",
            &digest,
            experiment_id,
            branch_id,
            now,
        )?;
        super::store::append_event(
            &transaction,
            EventInput {
                experiment_id,
                branch_id,
                operation_id,
                kind: "renamed",
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
