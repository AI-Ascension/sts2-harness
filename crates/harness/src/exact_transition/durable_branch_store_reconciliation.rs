// SPDX-License-Identifier: MIT

use super::store::SqliteBranchStore;
use super::validation::validate_label;
use super::{
    BranchStoreError, DurableBranch, DurableBranchStatus, MAX_BRANCH_PAGE,
    MAX_TRANSITION_LABEL_BYTES,
};

impl SqliteBranchStore {
    /// Lists branches needing owner-side startup reconciliation without changing their status.
    pub fn reconciliation_candidates(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<DurableBranch>, BranchStoreError> {
        let mut cursor = None;
        let mut candidates = Vec::new();
        loop {
            let page = self.list(experiment_id, cursor.as_deref(), MAX_BRANCH_PAGE)?;
            candidates.extend(page.branches.into_iter().filter(|branch| {
                matches!(
                    branch.status,
                    DurableBranchStatus::Pending
                        | DurableBranchStatus::Restoring
                        | DurableBranchStatus::Replaying
                        | DurableBranchStatus::Unknown
                )
            }));
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if cursor.as_deref() == Some(next_cursor.as_str()) {
                return Err(BranchStoreError::Corrupt);
            }
            cursor = Some(next_cursor);
        }
        Ok(candidates)
    }

    /// Deterministically resolves every half-created branch for one experiment after a restart.
    ///
    /// A fork intent that never started a strategy (`pending`) is archived, so it can be revived
    /// later through the reversible `archived -> pending` transition. A strategy that started but
    /// never reached `ready` (`restoring`/`replaying`) or whose effect is `unknown` fails closed as
    /// `failed` without replaying an uncertain effect. Each resolution uses the idempotent
    /// transition journal, so a retried reconciliation is a no-op and no branch is resolved twice.
    ///
    /// # Errors
    ///
    /// Returns [`BranchStoreError`] for an invalid operation prefix or any persistence failure.
    pub fn reconcile_startup(
        &self,
        operation_prefix: &str,
        experiment_id: &str,
    ) -> Result<Vec<DurableBranch>, BranchStoreError> {
        validate_label(operation_prefix, MAX_TRANSITION_LABEL_BYTES)?;
        let mut reconciled = Vec::new();
        for branch in self.reconciliation_candidates(experiment_id)? {
            let to = match branch.status {
                DurableBranchStatus::Pending => DurableBranchStatus::Archived,
                DurableBranchStatus::Restoring
                | DurableBranchStatus::Replaying
                | DurableBranchStatus::Unknown => DurableBranchStatus::Failed,
                _ => continue,
            };
            let operation_id = format!("{operation_prefix}.{}", branch.branch_id);
            reconciled.push(self.transition(
                &operation_id,
                experiment_id,
                &branch.branch_id,
                branch.metadata_revision,
                to,
            )?);
        }
        Ok(reconciled)
    }
}
