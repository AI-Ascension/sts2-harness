// SPDX-License-Identifier: MIT

use super::store::SqliteBranchStore;
use super::{BranchStoreError, DurableBranch, DurableBranchStatus, MAX_BRANCH_PAGE};

impl SqliteBranchStore {
    /// Lists branches needing owner-side startup reconciliation without changing their status.
    pub fn reconciliation_candidates(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<DurableBranch>, BranchStoreError> {
        let page = self.list(experiment_id, None, MAX_BRANCH_PAGE)?;
        Ok(page
            .branches
            .into_iter()
            .filter(|branch| {
                matches!(
                    branch.status,
                    DurableBranchStatus::Pending
                        | DurableBranchStatus::Restoring
                        | DurableBranchStatus::Replaying
                        | DurableBranchStatus::Unknown
                )
            })
            .collect())
    }
}
