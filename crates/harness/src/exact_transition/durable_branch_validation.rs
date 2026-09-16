// SPDX-License-Identifier: MIT

use rusqlite::{Transaction, params};

use super::{
    BranchArtifactRole, BranchAssurance, BranchStoreError, BranchStrategy, DurableBranchDraft,
    DurableBranchStatus, MAX_BRANCH_ARTIFACTS, MAX_BRANCH_NAME_BYTES, MAX_BRANCH_NOTES_BYTES,
    MAX_TRANSITION_LABEL_BYTES, OccurrenceId,
};
use crate::execution::ExactStateDigest;

pub(crate) fn validate_label(value: &str, max: usize) -> Result<(), BranchStoreError> {
    if value.is_empty()
        || value.len() > max
        || value.contains('\0')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-' | b'/')
        })
    {
        return Err(BranchStoreError::InvalidInput);
    }
    Ok(())
}

pub(crate) fn validate_operation_labels(
    operation_id: &str,
    experiment_id: &str,
    branch_id: &str,
) -> Result<(), BranchStoreError> {
    validate_label(operation_id, MAX_TRANSITION_LABEL_BYTES)?;
    validate_label(experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
    validate_label(branch_id, MAX_TRANSITION_LABEL_BYTES)
}

pub(crate) fn ensure_not_tombstoned(
    transaction: &Transaction<'_>,
    experiment_id: &str,
    branch_id: &str,
) -> Result<(), BranchStoreError> {
    let tombstoned: bool = transaction
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM branch_tombstones
                WHERE experiment_id = ?1 AND branch_id = ?2
            )",
            params![experiment_id, branch_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(BranchStoreError::persistence)?
        != 0;
    if tombstoned {
        return Err(BranchStoreError::InvalidTransition);
    }
    Ok(())
}

pub(crate) fn validate_optional_label(
    value: Option<&str>,
    max: usize,
) -> Result<(), BranchStoreError> {
    if let Some(value) = value {
        validate_label(value, max)?;
    }
    Ok(())
}

impl DurableBranchDraft {
    pub(crate) fn validate(&self) -> Result<(), BranchStoreError> {
        validate_label(&self.experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.root_branch_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.branch_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(self.parent_branch_id.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(self.fork.occurrence_id.as_str(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(
            self.fork
                .parent_occurrence_id
                .as_ref()
                .map(OccurrenceId::as_str),
            MAX_TRANSITION_LABEL_BYTES,
        )?;
        ExactStateDigest::parse(self.fork.state_digest.as_str())
            .map_err(|_| BranchStoreError::InvalidInput)?;
        validate_optional_label(self.source_handle.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(
            self.trajectory_prefix.as_deref(),
            MAX_TRANSITION_LABEL_BYTES,
        )?;
        validate_optional_label(self.effective_seed.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(self.setup_digest.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.boundary, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.run_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(self.episode_id.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(self.trajectory_id.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_optional_label(self.context_id.as_deref(), MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.policy_revision, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.config_revision, MAX_TRANSITION_LABEL_BYTES)?;
        validate_label(&self.name, MAX_BRANCH_NAME_BYTES)?;
        if let Some(notes) = &self.notes
            && (notes.is_empty() || notes.len() > MAX_BRANCH_NOTES_BYTES || notes.contains('\0'))
        {
            return Err(BranchStoreError::InvalidInput);
        }
        if self.parent_branch_id.is_none() && self.root_branch_id != self.branch_id {
            return Err(BranchStoreError::InvalidInput);
        }
        if self.parent_branch_id.as_deref() == Some(self.branch_id.as_str()) {
            return Err(BranchStoreError::InvalidInput);
        }
        if self.assurance != BranchAssurance::Unverified {
            return Err(BranchStoreError::InvalidInput);
        }
        if self.artifacts.len() > MAX_BRANCH_ARTIFACTS {
            return Err(BranchStoreError::Capacity);
        }
        let mut references = self.artifacts.clone();
        references.sort();
        references.dedup();
        if references.len() != self.artifacts.len() {
            return Err(BranchStoreError::Duplicate);
        }
        for artifact in &self.artifacts {
            validate_label(&artifact.artifact_id, MAX_TRANSITION_LABEL_BYTES)?;
        }
        Ok(())
    }
}

pub(crate) fn transition_allowed(from: DurableBranchStatus, to: DurableBranchStatus) -> bool {
    matches!(
        (from, to),
        (DurableBranchStatus::Pending, DurableBranchStatus::Restoring)
            | (DurableBranchStatus::Pending, DurableBranchStatus::Replaying)
            | (DurableBranchStatus::Pending, DurableBranchStatus::Archived)
            | (DurableBranchStatus::Ready, DurableBranchStatus::Restoring)
            | (DurableBranchStatus::Ready, DurableBranchStatus::Replaying)
            | (
                DurableBranchStatus::Restoring,
                DurableBranchStatus::Ready
                    | DurableBranchStatus::Failed
                    | DurableBranchStatus::Unknown
                    | DurableBranchStatus::Archived
            )
            | (
                DurableBranchStatus::Replaying,
                DurableBranchStatus::Ready
                    | DurableBranchStatus::Failed
                    | DurableBranchStatus::Unknown
                    | DurableBranchStatus::Archived
            )
            | (
                DurableBranchStatus::Ready,
                DurableBranchStatus::Running
                    | DurableBranchStatus::Held
                    | DurableBranchStatus::Archived
            )
            | (
                DurableBranchStatus::Running,
                DurableBranchStatus::Held
                    | DurableBranchStatus::Completed
                    | DurableBranchStatus::Failed
                    | DurableBranchStatus::Unknown
                    | DurableBranchStatus::Archived
            )
            | (
                DurableBranchStatus::Held,
                DurableBranchStatus::Running
                    | DurableBranchStatus::Completed
                    | DurableBranchStatus::Failed
                    | DurableBranchStatus::Unknown
                    | DurableBranchStatus::Archived
            )
            | (
                DurableBranchStatus::Completed,
                DurableBranchStatus::Archived
            )
            | (DurableBranchStatus::Failed, DurableBranchStatus::Archived)
            | (
                DurableBranchStatus::Unknown,
                DurableBranchStatus::Restoring
                    | DurableBranchStatus::Replaying
                    | DurableBranchStatus::Failed
                    | DurableBranchStatus::Archived
            )
            | (DurableBranchStatus::Archived, DurableBranchStatus::Pending)
    )
}

pub(crate) fn readiness_assured(strategy: BranchStrategy, assurance: BranchAssurance) -> bool {
    matches!(
        (strategy, assurance),
        (
            BranchStrategy::ExactRestore,
            BranchAssurance::ExactRestoreReceipt
        ) | (
            BranchStrategy::PrefixReplay,
            BranchAssurance::PrefixReplayBoundary
        )
    )
}

pub(crate) fn to_i64(value: u64) -> Result<i64, BranchStoreError> {
    i64::try_from(value).map_err(|_| BranchStoreError::InvalidInput)
}

#[allow(dead_code)]
fn _artifact_role_label(role: BranchArtifactRole) -> &'static str {
    role.as_str()
}
