// SPDX-License-Identifier: MIT

use super::store::SqliteBranchStore;
use super::{BranchStoreError, DurableBranch, DurableBranchStatus, MAX_BRANCH_PAGE};

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
}
