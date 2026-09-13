// SPDX-License-Identifier: MIT

use super::super::super::contract::{EventClassification, EventType, PendingOperation};
use super::super::{StoreCore, StoreError};
use super::{append_event, management_event, next_sequence};

pub(crate) fn record_operation_intent(
    core: &StoreCore,
    run_id: &str,
    expected_revision: u64,
    pending: PendingOperation,
) -> Result<(), StoreError> {
    core.mutate(|state| {
        let run = state
            .runs
            .get_mut(run_id)
            .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
        if run.snapshot.run_revision != expected_revision {
            return Err(StoreError::new(
                "stale_revision",
                "operation intent revision does not match the durable run",
            ));
        }
        if let Some(existing) = &run.snapshot.pending_operation {
            return if existing == &pending {
                Ok(())
            } else {
                Err(StoreError::new(
                    "operation_identity_conflict",
                    "a different pending operation is already durable",
                ))
            };
        }
        let mut snapshot = run.snapshot.clone();
        snapshot.pending_operation = Some(pending);
        let sequence = next_sequence(run)?;
        let event = management_event(
            &snapshot,
            sequence,
            EventType::OperationIntent,
            "live_operation_intent".to_owned(),
            EventClassification::Accepted,
        );
        append_event(run, event)?;
        run.snapshot = snapshot;
        Ok(())
    })
}
