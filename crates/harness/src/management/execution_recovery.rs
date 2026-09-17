// SPDX-License-Identifier: MIT

use super::super::super::contract::{
    CleanupState, RecoveryAdmission, RunSnapshot, WorkflowRunStatus,
};
use super::super::super::service::ManagementError;
use super::super::execution_records::{cleanup_session, lock_error};
use super::LiveWorkflowExecutionPort;

pub(super) fn admission(
    owner: &LiveWorkflowExecutionPort,
    snapshot: &RunSnapshot,
) -> Option<RecoveryAdmission> {
    if snapshot.pending_operation.is_some() {
        return Some(RecoveryAdmission::Reconciling);
    }
    if snapshot.admission.is_none()
        || !matches!(
            snapshot.status,
            WorkflowRunStatus::Running
                | WorkflowRunStatus::WaitingForProvider
                | WorkflowRunStatus::WaitingForGame
                | WorkflowRunStatus::Paused
        )
        || !matches!(
            snapshot.cleanup,
            CleanupState::NotStarted | CleanupState::Complete
        )
    {
        return None;
    }
    let runs = match owner.runs.lock() {
        Ok(runs) => runs,
        Err(_) => return Some(RecoveryAdmission::NeedsOperator),
    };
    if runs.contains_key(&snapshot.workflow_run_id) {
        Some(RecoveryAdmission::SafelyResumable {
            capability: "live.workflow.resume.v1".to_owned(),
        })
    } else {
        Some(RecoveryAdmission::NeedsOperator)
    }
}

pub(super) fn abort(
    owner: &LiveWorkflowExecutionPort,
    run_id: &str,
) -> Result<(), ManagementError> {
    let mut runs = owner.runs.lock().map_err(lock_error)?;
    let Some(run) = runs.remove(run_id) else {
        return Ok(());
    };
    drop(runs);
    let mut run = run.lock().map_err(lock_error)?;
    cleanup_session(&mut run, true)
}
