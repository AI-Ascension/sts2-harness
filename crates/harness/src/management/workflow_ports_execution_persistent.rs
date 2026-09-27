// SPDX-License-Identifier: MIT

//! The persistent (SQLite-backed) synthetic execution port, the controlled command application
//! the process driver exercises, and the run-status translation the in-memory port shares.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use std::sync::{Arc, Mutex};

use std::collections::BTreeMap;

use serde_json::json;

use super::auth::AuthContext;
use super::contract::{
    CommandKind, EventClassification, EventPayload, EventType, RunEvent, RunRequest,
    WorkflowRunStatus,
};
use super::service::{
    CommandApplication, CommandContext, ManagementError, RunAdmission, WorkflowExecutionPort,
};
use super::workflow_ports_definition::{parse_definition, raw_digest};
use super::workflow_ports_execution::{
    FixtureExecutor, StatusOverride, SyntheticRun, management_status, snapshot_from_runtime,
};
use super::workflow_ports_support::{lock_error, runtime_error, runtime_store_error};
use crate::workflow::{CompiledWorkflow, StrictRuntime};
use crate::{ContextBoundary, ControlAuthority, GateStatus};

/// Synthetic execution with a durable definition and runtime checkpoint. It exercises the
/// management process restart boundary while keeping gameplay effects explicitly unavailable.
pub(super) struct PersistentSyntheticExecutionPort {
    store: Arc<super::store::SqliteWorkflowStore>,
    runs: Mutex<BTreeMap<String, SyntheticRun>>,
}

impl PersistentSyntheticExecutionPort {
    pub(super) fn new(store: Arc<super::store::SqliteWorkflowStore>) -> Self {
        Self {
            store,
            runs: Mutex::new(BTreeMap::new()),
        }
    }

    fn restore_run(&self, run_id: &str) -> Result<SyntheticRun, ManagementError> {
        let record = self
            .store
            .load_runtime(run_id)
            .map_err(runtime_store_error)?
            .ok_or_else(|| {
                ManagementError::unresolved(
                    "runtime_after_restart",
                    "durable runtime checkpoint is unavailable after service restart",
                )
            })?;
        let definition = parse_definition(&record.definition)?;
        let compiled = CompiledWorkflow::compile(definition)
            .map_err(|error| ManagementError::invalid("definition_compile", error.to_string()))?;
        let runtime =
            StrictRuntime::from_snapshot(compiled, record.snapshot).map_err(runtime_error)?;
        let control = record
            .control_journal
            .as_deref()
            .map(ControlAuthority::recover)
            .transpose()
            .map_err(control_error)?;
        Ok(SyntheticRun {
            runtime,
            executor: FixtureExecutor,
            definition: record.definition,
            definition_digest: record.definition_digest,
            cancelled: record.cancelled,
            control,
        })
    }

    fn persist_run(&self, run_id: &str, run: &SyntheticRun) -> Result<(), ManagementError> {
        let control_journal = run
            .control
            .as_ref()
            .map(ControlAuthority::export_journal)
            .transpose()
            .map_err(control_error)?;
        self.store
            .save_runtime(
                run_id,
                &run.definition_digest,
                &run.definition,
                &run.runtime.snapshot(),
                run.cancelled,
                control_journal.as_deref(),
            )
            .map_err(runtime_store_error)
    }
}

impl WorkflowExecutionPort for PersistentSyntheticExecutionPort {
    fn submit(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        let definition = request.definition.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "artifact_port_unavailable",
                "synthetic execution requires an inline workflow definition",
            )
        })?;
        let parsed = parse_definition(definition)?;
        let compiled = CompiledWorkflow::compile(parsed)
            .map_err(|error| ManagementError::invalid("definition_compile", error.to_string()))?;
        let runtime = StrictRuntime::new(compiled)
            .map_err(|error| ManagementError::invalid("runtime_admission", error.to_string()))?;
        let run_id = format!(
            "run.synthetic.{}",
            &raw_digest(&json!({
                "request_id": request.request_id,
                "instance_id": request.instance_id,
                "definition_digest": definition_digest,
            }))?[..32]
        );
        let run = SyntheticRun {
            runtime,
            executor: FixtureExecutor,
            definition: definition.clone(),
            definition_digest: definition_digest.to_owned(),
            cancelled: false,
            control: Some(ControlAuthority::new(
                synthetic_control_boundary(&run_id),
                "revision-1",
            )),
        };
        self.persist_run(&run_id, &run)?;
        let snapshot = snapshot_from_runtime(
            &run_id,
            definition_digest,
            &run.runtime,
            1,
            false,
            request.admission.clone(),
        );
        let event = admission_event(&run_id, definition_digest);
        self.runs.lock().map_err(lock_error)?.insert(run_id, run);
        Ok(RunAdmission {
            snapshot,
            initial_events: vec![event],
        })
    }

    fn apply_command(
        &self,
        context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        let mut runs = self.runs.lock().map_err(lock_error)?;
        if !runs.contains_key(&context.request.run_id) {
            let restored = self.restore_run(&context.request.run_id)?;
            runs.insert(context.request.run_id.clone(), restored);
        }
        let run = runs.get_mut(&context.request.run_id).ok_or_else(|| {
            ManagementError::unresolved(
                "runtime_after_restart",
                "durable runtime checkpoint is unavailable after service restart",
            )
        })?;
        let status = match context.request.kind {
            CommandKind::Pause | CommandKind::Resume => apply_controlled_runtime_command(
                run,
                &context.request.kind,
                &context.request.command_id,
            )?,
            CommandKind::Step | CommandKind::Cancel => {
                apply_runtime_command(run, &context.request.kind)?
            }
        };
        let revision = context
            .snapshot
            .run_revision
            .checked_add(1)
            .ok_or_else(|| ManagementError::budget("revision_overflow", "revision overflowed"))?;
        self.persist_run(&context.request.run_id, run)?;
        let snapshot = snapshot_from_runtime(
            &context.request.run_id,
            &run.definition_digest,
            &run.runtime,
            revision,
            run.cancelled,
            context.snapshot.admission.clone(),
        )
        .with_status(status);
        Ok(CommandApplication {
            snapshot,
            outcome: super::contract::CommandOutcome::Applied,
            reason_code: context.request.kind.reason_code().to_owned(),
            context_binding: None,
        })
    }
}

pub(super) fn apply_controlled_runtime_command(
    run: &mut SyntheticRun,
    kind: &CommandKind,
    command_id: &str,
) -> Result<WorkflowRunStatus, ManagementError> {
    match kind {
        CommandKind::Pause => {
            let authority = run.control.as_mut().ok_or_else(|| {
                ManagementError::unavailable(
                    "context_control_recovery_unavailable",
                    "durable control journal is unavailable for this legacy run",
                )
            })?;
            authority
                .request_pause(command_id, authority.state().control_version)
                .map_err(control_error)?;
            if authority.state().status != GateStatus::PausedReady {
                return Err(ManagementError::unresolved(
                    "context_pause_not_ready",
                    "context authority has unresolved operations",
                ));
            }
            run.runtime.pause().map_err(runtime_error)?;
            Ok(WorkflowRunStatus::Paused)
        }
        CommandKind::Resume => {
            let authority = run.control.as_mut().ok_or_else(|| {
                ManagementError::unavailable(
                    "context_control_recovery_unavailable",
                    "durable control journal is unavailable for this legacy run",
                )
            })?;
            let boundary = authority.state().boundary.clone();
            authority
                .resume(command_id, authority.state().control_version, &boundary)
                .map_err(control_error)?;
            run.runtime.resume().map_err(runtime_error)?;
            Ok(WorkflowRunStatus::Running)
        }
        CommandKind::Step | CommandKind::Cancel => apply_runtime_command(run, kind),
    }
}

pub(super) fn apply_runtime_command(
    run: &mut SyntheticRun,
    kind: &CommandKind,
) -> Result<WorkflowRunStatus, ManagementError> {
    match kind {
        CommandKind::Pause => {
            run.runtime.pause().map_err(runtime_error)?;
        }
        CommandKind::Resume => {
            run.runtime.resume().map_err(runtime_error)?;
        }
        CommandKind::Step => {
            run.runtime.step(&mut run.executor).map_err(runtime_error)?;
        }
        CommandKind::Cancel => run.cancelled = true,
    }
    Ok(management_status(run.runtime.status(), run.cancelled))
}

pub(super) fn synthetic_control_boundary(run_id: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: run_id.to_owned(),
        episode_id: format!("{run_id}.episode"),
        agent_id: "synthetic.agent".to_owned(),
        state_id: "synthetic.workflow.state".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "synthetic.workflow.adapter.v1".to_owned(),
        model_revision: "synthetic.workflow.model.v1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 0,
    }
}

pub(super) fn control_error(code: String) -> ManagementError {
    match code.as_str() {
        "stale_control_version" | "already_paused" | "not_ready" | "preview_stale" => {
            ManagementError::conflict("context_control_conflict", code)
        }
        "obsolete_plan" | "run_not_ready" => {
            ManagementError::unresolved("context_control_unresolved", code)
        }
        _ => ManagementError::invalid("context_control_invalid", code),
    }
}

pub(super) fn admission_event(run_id: &str, definition_digest: &str) -> RunEvent {
    RunEvent {
        schema_version: super::contract::EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        sequence: 1,
        run_revision: 1,
        event_type: EventType::RunStarted,
        definition_digest: definition_digest.to_owned(),
        node_execution_id: "synthetic.admission".to_owned(),
        payload: EventPayload {
            operation_id: None,
            classification: Some(EventClassification::Accepted),
            reason_code: "synthetic_admitted".to_owned(),
        },
        integrity_digest: None,
    }
}
