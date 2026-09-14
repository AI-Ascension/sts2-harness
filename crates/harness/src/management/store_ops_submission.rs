// SPDX-License-Identifier: MIT

use super::super::{StoreCore, StoreError};
use crate::management::contract::RunSnapshot;

pub(crate) fn update_run_snapshot(
    core: &StoreCore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
) -> Result<(), StoreError> {
    core.mutate(|state| {
        let submission = state
            .submissions
            .get(request_id)
            .ok_or_else(|| StoreError::new("run_not_found", "workflow submission was not found"))?;
        if submission.request_digest != request_digest {
            return Err(StoreError::new(
                "submission_conflict",
                "workflow submission digest does not match the reserved request",
            ));
        }
        let run = state
            .runs
            .get_mut(&submission.workflow_run_id)
            .ok_or_else(|| {
                StoreError::new(
                    "store_corrupt",
                    "submission index points to a missing workflow run",
                )
            })?;
        if run.snapshot.workflow_run_id != snapshot.workflow_run_id
            || run.snapshot.definition_digest != snapshot.definition_digest
            || run.snapshot.run_revision != snapshot.run_revision
            || run.snapshot.admission != snapshot.admission
        {
            return Err(StoreError::new(
                "snapshot_identity_conflict",
                "reserved workflow identity or target admission changed",
            ));
        }
        run.snapshot = snapshot;
        Ok(())
    })
}
