// SPDX-License-Identifier: MIT

use serde_json::json;

use super::super::contract::{
    Budget, CleanupState, CommandOutcome, Cursor, ExecutionMode, GameOutcome, PendingOperation,
    RunRequest, RunSnapshot, TargetAdmissionBinding, WorkflowRunStatus,
};
use super::super::service::{CommandApplication, ManagementError};
use super::execution_state::LiveRun;
use crate::workflow::{RuntimeFault, RuntimeStatus, StrictRuntime};

pub(super) struct SnapshotState {
    pub(super) cancelled: bool,
    pub(super) pending: Option<PendingOperation>,
    pub(super) provider_calls: u64,
    pub(super) cleanup: CleanupState,
    pub(super) admission: Option<TargetAdmissionBinding>,
}

impl Default for SnapshotState {
    fn default() -> Self {
        Self {
            cancelled: false,
            pending: None,
            provider_calls: 0,
            cleanup: CleanupState::NotStarted,
            admission: None,
        }
    }
}

pub(super) fn application(
    run: &LiveRun,
    status: WorkflowRunStatus,
    outcome: CommandOutcome,
    reason: &str,
    revision: u64,
) -> CommandApplication {
    let mut snapshot = snapshot_from_runtime(
        &run.run_id,
        &run.definition_digest,
        &run.runtime,
        SnapshotState {
            cancelled: run.cancelled,
            pending: run
                .state
                .pending
                .as_ref()
                .map(|pending| PendingOperation {
                    operation_id: pending.identity.operation_id.clone(),
                    classification: pending.state.clone(),
                    instance_id: run.instance_id.clone(),
                    original_generation: pending.identity.generation,
                    payload_digest: crate::sha256_hex(pending.action.action_id()),
                })
                .or_else(|| {
                    // A held decision attempt is an in-flight unknown effect: publishing it the way
                    // the action path publishes an intent makes recovery admission short-circuit to
                    // `Reconciling` and `authority.recovery` report `pending_effect_visible`, instead
                    // of leaving the run looking like a state with nothing outstanding.
                    run.state
                        .pending_decision
                        .as_ref()
                        .map(|held| super::node_decision::decision_intent(held, &run.instance_id))
                }),
            provider_calls: run.state.provider_calls,
            cleanup: run.cleanup.clone(),
            admission: run.admission.clone(),
        },
        revision,
    );
    snapshot.status = status;
    CommandApplication {
        snapshot,
        outcome,
        reason_code: reason.to_owned(),
        context_binding: run.context_binding.clone(),
    }
}

pub(super) fn cleanup_session(run: &mut LiveRun, stop: bool) -> Result<(), ManagementError> {
    run.cleanup = CleanupState::Pending;
    let mut failure = None;
    if stop && let Err(error) = run.state.session.stop_episode() {
        failure = Some(error);
    }
    if let Err(error) = run.state.session.release_lease()
        && failure.is_none()
    {
        failure = Some(error);
    }
    run.cleanup = if failure.is_some() {
        CleanupState::NeedsOperator
    } else {
        CleanupState::Complete
    };
    failure.map_or(Ok(()), Err)
}

pub(super) fn snapshot_from_runtime(
    run_id: &str,
    definition_digest: &str,
    runtime: &StrictRuntime,
    state: SnapshotState,
    revision: u64,
) -> RunSnapshot {
    let current = runtime.snapshot();
    RunSnapshot {
        schema_version: super::super::contract::RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        definition_digest: definition_digest.to_owned(),
        run_revision: revision,
        status: management_status(runtime.status(), state.cancelled),
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: current.graph_id.as_str().to_owned(),
            node_id: current.node_id.as_str().to_owned(),
            node_execution_id: format!("live.node.{}", current.event_sequence.saturating_add(1)),
        },
        pending_operation: state.pending,
        budget: Budget {
            provider_calls_consumed: state.provider_calls,
            node_steps_consumed: current.steps,
            ..Budget::default()
        },
        cleanup: state.cleanup,
        admission: state.admission,
        execution_mode: Some(ExecutionMode::Live),
    }
}

/// Deterministically binds a live runtime configuration to the admitted
/// management submission. The runtime, provider-policy owner, and context
/// authority must all use this exact namespace rather than a caller alias.
pub fn live_run_id(
    request: &RunRequest,
    definition_digest: &str,
) -> Result<String, ManagementError> {
    let value = json!({
        "request_id": request.request_id,
        "instance_id": request.instance_id,
        "definition_digest": definition_digest,
    });
    let digest = super::super::workflow_ports::raw_digest(&value)?;
    Ok(format!("run.live.{}", &digest[..32]))
}

pub(super) fn management_status(status: RuntimeStatus, cancelled: bool) -> WorkflowRunStatus {
    if cancelled {
        return WorkflowRunStatus::Cancelled;
    }
    match status {
        RuntimeStatus::Running => WorkflowRunStatus::Running,
        RuntimeStatus::Paused => WorkflowRunStatus::Paused,
        RuntimeStatus::Completed => WorkflowRunStatus::Completed,
        RuntimeStatus::Failed => WorkflowRunStatus::Failed,
        RuntimeStatus::NeedsOperator => WorkflowRunStatus::NeedsOperator,
    }
}

pub(super) fn runtime_error(error: RuntimeFault) -> ManagementError {
    match error {
        RuntimeFault::BudgetExceeded => {
            ManagementError::budget("live_runtime_budget", error.to_string())
        }
        RuntimeFault::UnknownEffect => {
            ManagementError::unresolved("live_operation_unknown", error.to_string())
        }
        RuntimeFault::ExecutorUnavailable => {
            ManagementError::capability("live_node_unavailable", error.to_string())
        }
        RuntimeFault::ExecutorRejected | RuntimeFault::TypeMismatch => {
            ManagementError::invalid("live_node_rejected", error.to_string())
        }
        _ => ManagementError::conflict("live_runtime_state", error.to_string()),
    }
}

pub(super) fn lock_error<T>(_: std::sync::PoisonError<T>) -> ManagementError {
    ManagementError::store(
        "live_runtime_lock",
        "live workflow runtime lock is poisoned",
    )
}
