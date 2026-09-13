// SPDX-License-Identifier: MIT

use super::super::contract::{CommandKind, CommandOutcome, PendingOperation, WorkflowRunStatus};
use super::super::service::{CommandApplication, CommandContext, ManagementError};
use crate::workflow::{RuntimeFault, RuntimeStatus};

use super::execution_records::{application, cleanup_session, management_status, runtime_error};
use super::node::LiveNodeExecutor;
use super::node_recovery::reconcile_pending;

pub(super) fn apply_command(
    owner: &super::execution::LiveWorkflowExecutionPort,
    context: CommandContext,
    record_intent: Option<&dyn Fn(PendingOperation) -> Result<(), ManagementError>>,
) -> Result<CommandApplication, ManagementError> {
    let mut runs = owner
        .runs()
        .lock()
        .map_err(super::execution_records::lock_error)?;
    let run = runs.get_mut(&context.request.run_id).ok_or_else(|| {
        ManagementError::unresolved(
            "live_runtime_after_restart",
            "live session is unavailable after service restart",
        )
    })?;
    if run.definition_digest != context.snapshot.definition_digest {
        return Err(ManagementError::conflict(
            "live_identity_mismatch",
            "live command is bound to a different workflow definition",
        ));
    }
    let revision = context.snapshot.run_revision.saturating_add(1);
    let (status, outcome, reason) = match context.request.kind {
        CommandKind::Pause => {
            if run.cancelled {
                return Ok(application(
                    run,
                    WorkflowRunStatus::Cancelled,
                    CommandOutcome::Applied,
                    "cancel_dominates",
                    revision,
                ));
            }
            run.state.session.pause()?;
            run.runtime.pause().map_err(runtime_error)?;
            (WorkflowRunStatus::Paused, CommandOutcome::Applied, "pause")
        }
        CommandKind::Resume => {
            if run.cancelled {
                (
                    WorkflowRunStatus::Cancelled,
                    CommandOutcome::Applied,
                    "cancel_dominates",
                )
            } else {
                run.state.session.resume()?;
                run.runtime.resume().map_err(runtime_error)?;
                (
                    WorkflowRunStatus::Running,
                    CommandOutcome::Applied,
                    "resume",
                )
            }
        }
        CommandKind::Cancel => {
            let reconcile_error = reconcile_pending(&mut run.state).err();
            let cleanup_error = cleanup_session(run, true).err();
            run.cancelled = true;
            if let Some(error) = reconcile_error
                && error.class != super::super::contract::ErrorClass::Unresolved
            {
                return Err(error);
            }
            if cleanup_error.is_some() {
                (
                    WorkflowRunStatus::NeedsOperator,
                    CommandOutcome::Applied,
                    "live_cleanup_failed",
                )
            } else {
                (
                    WorkflowRunStatus::Cancelled,
                    CommandOutcome::Applied,
                    "cancel",
                )
            }
        }
        CommandKind::Step => {
            if run.cancelled {
                (
                    WorkflowRunStatus::Cancelled,
                    CommandOutcome::Applied,
                    "cancel_dominates",
                )
            } else if run.runtime.status() == RuntimeStatus::Paused {
                return Err(ManagementError::conflict(
                    "live_run_paused",
                    "step requires a resumed workflow",
                ));
            } else {
                reconcile_pending(&mut run.state)?;
                if run.runtime.status() == RuntimeStatus::NeedsOperator {
                    run.runtime
                        .resume_after_unknown_effect()
                        .map_err(runtime_error)?;
                }
                let result = {
                    let mut executor = LiveNodeExecutor {
                        state: &mut run.state,
                        intent_recorder: record_intent,
                    };
                    run.runtime.step(&mut executor)
                };
                match result {
                    Ok(status) => {
                        let status = management_status(status, false);
                        if run.runtime.status().terminal()
                            && run.state.pending.is_none()
                            && cleanup_session(run, false).is_err()
                        {
                            return Ok(application(
                                run,
                                WorkflowRunStatus::NeedsOperator,
                                CommandOutcome::Applied,
                                "live_cleanup_failed",
                                revision,
                            ));
                        }
                        (status, CommandOutcome::Applied, "step")
                    }
                    Err(RuntimeFault::UnknownEffect) => {
                        let application = application(
                            run,
                            WorkflowRunStatus::NeedsOperator,
                            CommandOutcome::Pending,
                            "live_operation_unknown",
                            revision,
                        );
                        return Ok(application);
                    }
                    Err(error) => {
                        let needs_operator = error == RuntimeFault::BudgetExceeded;
                        if !needs_operator {
                            run.runtime.fail();
                        }
                        let _ = cleanup_session(run, true);
                        let status = if needs_operator {
                            WorkflowRunStatus::NeedsOperator
                        } else if run.cleanup == super::super::contract::CleanupState::Complete {
                            WorkflowRunStatus::Failed
                        } else {
                            WorkflowRunStatus::NeedsOperator
                        };
                        return Ok(application(
                            run,
                            status,
                            CommandOutcome::Applied,
                            "live_execution_failed",
                            revision,
                        ));
                    }
                }
            }
        }
    };
    Ok(application(run, status, outcome, reason, revision))
}
