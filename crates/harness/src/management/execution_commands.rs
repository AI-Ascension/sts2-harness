// SPDX-License-Identifier: MIT

use super::super::context_owner::{ContextBindingRequest, compose_context_owner_binding};
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
    // Evidence belongs only to this command, never a later step/pause/cancel.
    run.context_binding = None;
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
                bind_dispatch_context(run, &context)?;
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

/// Binds the authoritative context owner to the invocation the runtime is about
/// to execute when the cursor is on a context-bound node.
///
/// The request identity is derived from the runtime's own cursor allocation
/// (`live.node.{event_sequence + 1}`, matching `snapshot_from_runtime`), and the
/// returned binding is validated against the persisted run snapshot. A binding
/// for a different invocation therefore fails closed instead of being accepted
/// while the run may never execute it.
fn bind_dispatch_context(
    run: &mut super::execution::LiveRun,
    context: &CommandContext,
) -> Result<(), ManagementError> {
    let runtime_snapshot = run.runtime.snapshot();
    let graph_id = runtime_snapshot.graph_id.as_str();
    let node_id = runtime_snapshot.node_id.as_str();
    let Some((node_kind, context_ref)) = run
        .context_nodes
        .iter()
        .find(|node| node.graph_id == graph_id && node.node_id == node_id)
        .map(|node| (node.node_kind.clone(), node.context_ref.clone()))
    else {
        return Ok(());
    };
    let node_execution_id = format!(
        "live.node.{}",
        runtime_snapshot.event_sequence.saturating_add(1)
    );
    let owner = &context.context_owner;
    if !owner.is_available() {
        return Err(ManagementError::unavailable(
            "context_owner_unavailable",
            "live context binding requires an attached authoritative context owner",
        ));
    }
    let catalog = owner.catalog(&context.actor)?;
    catalog.validate()?;
    let descriptor = catalog.descriptor_for(context_ref.as_str(), node_kind.as_str())?;
    if !descriptor.grants.metadata_read {
        return Err(ManagementError::capability(
            "context_binding_metadata_unavailable",
            "context owner catalog does not grant metadata access for this node",
        ));
    }
    let request = ContextBindingRequest {
        workflow_run_id: run.run_id.clone(),
        definition_digest: run.definition_digest.clone(),
        instance_id: run.instance_id.clone(),
        graph_id: graph_id.to_owned(),
        node_id: node_id.to_owned(),
        node_execution_id,
        node_kind,
        context_ref,
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    request.validate()?;
    let binding = owner.bind(&context.actor, &request)?;
    binding.validate_for_request(&request)?;
    // Admission and the observable owner surface compose the binding with its
    // admitting descriptor through the same fail-closed seam.
    compose_context_owner_binding(&catalog, &binding)?;
    binding.validate(Some(&context.snapshot))?;
    run.context_binding = Some(binding);
    Ok(())
}
