// SPDX-License-Identifier: MIT

use serde_json::json;

use super::super::contract::{
    Budget, CleanupState, CommandOutcome, Cursor, GameOutcome, RunRequest, RunSnapshot,
    WorkflowRunStatus,
};
use super::super::service::{CommandApplication, ManagementError};
use super::execution::{LiveRun, PendingDispatch};
use crate::workflow::{RuntimeFault, RuntimeStatus, StrictRuntime};

pub(super) fn application(
    run: &LiveRun,
    status: WorkflowRunStatus,
    outcome: CommandOutcome,
    reason: &str,
    revision: u64,
) -> CommandApplication {
    let mut snapshot = snapshot_from_runtime(
        &run.run_id,
        &run.instance_id,
        &run.definition_digest,
        &run.runtime,
        run.cancelled,
        run.state.pending.as_ref(),
        revision,
    );
    snapshot.status = status;
    CommandApplication {
        snapshot,
        outcome,
        reason_code: reason.to_owned(),
    }
}

pub(super) fn snapshot_from_runtime(
    run_id: &str,
    instance_id: &str,
    definition_digest: &str,
    runtime: &StrictRuntime,
    cancelled: bool,
    pending: Option<&PendingDispatch>,
    revision: u64,
) -> RunSnapshot {
    let current = runtime.snapshot();
    RunSnapshot {
        schema_version: super::super::contract::RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        definition_digest: definition_digest.to_owned(),
        run_revision: revision,
        status: management_status(runtime.status(), cancelled),
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: current.graph_id.as_str().to_owned(),
            node_id: current.node_id.as_str().to_owned(),
            node_execution_id: format!("live.node.{}", current.event_sequence.saturating_add(1)),
        },
        pending_operation: pending.map(|item| super::super::contract::PendingOperation {
            operation_id: item.identity.operation_id.clone(),
            classification: item.state.clone(),
            instance_id: instance_id.to_owned(),
            original_generation: item.identity.generation,
            payload_digest: crate::sha256_hex(item.action.action_id()),
        }),
        budget: Budget {
            node_steps_consumed: current.steps,
            ..Budget::default()
        },
        cleanup: CleanupState::NotStarted,
    }
}

pub(super) fn live_run_id(
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
