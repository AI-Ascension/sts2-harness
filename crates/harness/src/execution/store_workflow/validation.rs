// SPDX-License-Identifier: MIT

use super::super::types::ExecutionStoreError;
use super::super::workflow_types::{WorkflowEvent, WorkflowEventPayload};
use super::RunRow;

pub(super) fn validate_event_run(
    row: &RunRow,
    event: &WorkflowEvent,
) -> Result<(), ExecutionStoreError> {
    match &event.payload {
        WorkflowEventPayload::RunStarted { .. } => Err(ExecutionStoreError::Conflict),
        WorkflowEventPayload::InvocationReserved { .. }
        | WorkflowEventPayload::InvocationCompleted { .. } => Err(ExecutionStoreError::Conflict),
        WorkflowEventPayload::CursorAdvanced { .. }
        | WorkflowEventPayload::RunCompleted
        | WorkflowEventPayload::RunFailed => {
            let workflow_id =
                super::super::workflow_types::WorkflowDefinitionId::new(row.workflow_id.clone())?;
            let plan_id = super::super::workflow_types::WorkflowPlanId::new(row.plan_id.clone())?;
            let episode_id =
                super::super::workflow_types::WorkflowEpisodeId::new(row.episode_id.clone())?;
            if workflow_id.as_str().is_empty()
                || plan_id.as_str().is_empty()
                || episode_id.as_str().is_empty()
            {
                return Err(ExecutionStoreError::Corrupt);
            }
            Ok(())
        }
    }
}
