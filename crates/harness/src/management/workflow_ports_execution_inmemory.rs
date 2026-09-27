// SPDX-License-Identifier: MIT

//! The in-memory synthetic execution port.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use serde_json::json;

use super::auth::AuthContext;
use super::contract::{EventClassification, EventPayload, EventType, RunEvent, RunRequest};
use super::service::{
    CommandApplication, CommandContext, ManagementError, RunAdmission, WorkflowExecutionPort,
};
use super::workflow_ports_definition::{parse_definition, raw_digest};
use super::workflow_ports_execution::{
    FixtureExecutor, StatusOverride, SyntheticExecutionPort, SyntheticRun, snapshot_from_runtime,
};
use super::workflow_ports_execution_persistent::{
    apply_controlled_runtime_command, synthetic_control_boundary,
};
use super::workflow_ports_support::lock_error;
use crate::ControlAuthority;
use crate::workflow::{CompiledWorkflow, StrictRuntime};

impl WorkflowExecutionPort for SyntheticExecutionPort {
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
        let snapshot = snapshot_from_runtime(
            &run_id,
            definition_digest,
            &runtime,
            1,
            false,
            request.admission.clone(),
        );
        let event = RunEvent {
            schema_version: super::contract::EVENT_SCHEMA_VERSION.to_owned(),
            workflow_run_id: run_id.clone(),
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
        };
        self.runs.lock().map_err(lock_error)?.insert(
            run_id.clone(),
            SyntheticRun {
                runtime,
                executor: FixtureExecutor,
                definition: definition.clone(),
                definition_digest: definition_digest.to_owned(),
                cancelled: false,
                control: Some(ControlAuthority::new(
                    synthetic_control_boundary(&run_id),
                    "revision-1",
                )),
            },
        );
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
        let run = runs.get_mut(&context.request.run_id).ok_or_else(|| {
            ManagementError::unresolved(
                "runtime_after_restart",
                "synthetic runtime state is unavailable after service restart",
            )
        })?;
        let status = apply_controlled_runtime_command(
            run,
            &context.request.kind,
            &context.request.command_id,
        )?;
        let revision = context
            .snapshot
            .run_revision
            .checked_add(1)
            .ok_or_else(|| {
                ManagementError::budget("revision_overflow", "run revision overflowed")
            })?;
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
