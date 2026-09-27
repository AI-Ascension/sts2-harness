// SPDX-License-Identifier: MIT

//! Shared execution-port scaffolding: the fixture run, the node executor, and the runtime
//! snapshot projection both execution ports build their results from.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;

use super::contract::{
    Budget, CleanupState, Cursor, ExecutionMode, GameOutcome, RunSnapshot, TargetAdmissionBinding,
    WorkflowRunStatus,
};
use crate::ControlAuthority;
use crate::workflow::{
    NodeExecutor, NodeOutcome, RuntimeContext, RuntimeFault, RuntimeStatus, StrictRuntime,
    TypedValue,
};

#[derive(Default)]
pub(super) struct SyntheticExecutionPort {
    pub(super) runs: Mutex<BTreeMap<String, SyntheticRun>>,
}

pub(super) struct SyntheticRun {
    pub(super) runtime: StrictRuntime,
    pub(super) executor: FixtureExecutor,
    pub(super) definition: Value,
    pub(super) definition_digest: String,
    pub(super) cancelled: bool,
    /// Present only for the in-memory synthetic execution path. It is the same
    /// Harness control authority that fences its pause/resume commands; the
    /// Context Console reducer is never consulted here.
    pub(super) control: Option<ControlAuthority>,
}

#[derive(Default)]
pub(super) struct FixtureExecutor;

impl NodeExecutor for FixtureExecutor {
    fn execute(
        &mut self,
        _node: &crate::workflow::NodeDefinition,
        _context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        Ok(NodeOutcome::new(
            crate::workflow::EdgeOutcome::Ok,
            TypedValue::Unknown,
        ))
    }
}

pub(super) fn snapshot_from_runtime(
    run_id: &str,
    definition_digest: &str,
    runtime: &StrictRuntime,
    revision: u64,
    cancelled: bool,
    admission: Option<TargetAdmissionBinding>,
) -> RunSnapshot {
    let runtime_snapshot = runtime.snapshot();
    RunSnapshot {
        schema_version: super::contract::RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        definition_digest: definition_digest.to_owned(),
        run_revision: revision,
        status: management_status(runtime.status(), cancelled),
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: runtime_snapshot.graph_id.as_str().to_owned(),
            node_id: runtime_snapshot.node_id.as_str().to_owned(),
            node_execution_id: format!("synthetic.{}", runtime_snapshot.event_sequence),
        },
        pending_operation: None,
        budget: Budget {
            node_steps_consumed: runtime_snapshot.steps,
            ..Budget::default()
        },
        cleanup: CleanupState::NotStarted,
        admission,
        execution_mode: Some(ExecutionMode::Synthetic),
    }
}

pub(super) trait StatusOverride {
    fn with_status(self, status: WorkflowRunStatus) -> Self;
}

impl StatusOverride for RunSnapshot {
    fn with_status(mut self, status: WorkflowRunStatus) -> Self {
        self.status = status;
        self
    }
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
