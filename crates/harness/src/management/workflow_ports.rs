// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::auth::AuthContext;
use super::contract::{
    Budget, CleanupState, CommandKind, Cursor, Diagnostic, DiagnosticSeverity, EventClassification,
    EventPayload, EventType, GameOutcome, RunEvent, RunRequest, RunSnapshot, WorkflowRunStatus,
};
use super::service::{
    CapabilityPort, CommandApplication, CommandContext, DefinitionPort, DiffResult,
    InspectionResult, ManagementError, ReplayResult, RunAdmission, ValidationResult,
    WorkflowExecutionPort, WorkflowReplayPort,
};
use crate::workflow::{
    CompiledWorkflow, DecodeError, NodeExecutor, NodeOutcome, RuntimeContext, RuntimeFault,
    RuntimeStatus, StrictRuntime, TypedValue, WorkflowDefinition, decode_strict, semantic_diff,
    validate_definition,
};

/// Builds the authenticated synthetic service used by the CLI and process driver.
/// The adapter owns no gameplay authority and returns only deterministic fixture
/// outcomes through the same strict runtime boundary used by component tests.
pub fn synthetic_file_store(
    store: super::store::FileWorkflowStore,
) -> super::service::ManagementService {
    super::service::ManagementService::file_store(store)
        .with_definition_port(Arc::new(SyntheticDefinitionPort))
        .with_execution_port(Arc::new(SyntheticExecutionPort::default()))
        .with_replay_port(Arc::new(SyntheticReplayPort))
        .with_capability_port(Arc::new(SyntheticCapabilityPort))
}

struct SyntheticDefinitionPort;

impl DefinitionPort for SyntheticDefinitionPort {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        let parsed = parse_definition(definition)?;
        let digest = raw_digest(definition)?;
        let mut diagnostics = Vec::new();
        diagnostics.extend(capability_diagnostics(&parsed, capabilities)?);
        Ok(ValidationResult {
            definition_digest: digest,
            diagnostics,
        })
    }

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        let parsed = parse_definition(definition)?;
        Ok(InspectionResult {
            definition_digest: raw_digest(definition)?,
            workflow_id: Some(parsed.workflow_id.as_str().to_owned()),
            workflow_version: Some(parsed.version.as_str().to_owned()),
            required_capabilities: parsed
                .capabilities
                .required
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            graph_count: parsed.graphs.len() as u64,
            node_count: parsed
                .graphs
                .iter()
                .map(|graph| graph.nodes.len() as u64)
                .sum(),
        })
    }

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        let old = parse_definition(old_definition)?;
        let new = parse_definition(new_definition)?;
        let change = semantic_diff(&old, &new).map_err(|error| {
            ManagementError::invalid("canonicalization_failed", error.to_string())
        })?;
        Ok(DiffResult {
            old_definition_digest: raw_digest(old_definition)?,
            new_definition_digest: raw_digest(new_definition)?,
            semantic_change: change.executable_changed,
            changed_paths: if change.executable_changed {
                vec!["/".to_owned()]
            } else if change.annotations_changed {
                vec!["/annotations".to_owned()]
            } else {
                Vec::new()
            },
        })
    }
}

fn parse_definition(value: &Value) -> Result<WorkflowDefinition, ManagementError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ManagementError::invalid("definition_encode", error.to_string()))?;
    let definition: WorkflowDefinition = decode_strict(&bytes).map_err(decode_management_error)?;
    validate_definition(&definition).map_err(|error| {
        ManagementError::invalid(
            "definition_invalid",
            format!("workflow definition validation failed: {error}"),
        )
    })?;
    Ok(definition)
}

fn decode_management_error(error: DecodeError) -> ManagementError {
    ManagementError::invalid("definition_decode", error.to_string())
}

fn raw_digest(value: &Value) -> Result<String, ManagementError> {
    super::contract::digest_value(value).map_err(ManagementError::from)
}

fn capability_diagnostics(
    definition: &WorkflowDefinition,
    manifest: &Value,
) -> Result<Vec<Diagnostic>, ManagementError> {
    let available = manifest
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ManagementError::invalid(
                "capability_manifest",
                "capability manifest must contain a capabilities array",
            )
        })?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    Ok(definition
        .capabilities
        .required
        .iter()
        .filter(|required| !available.contains(required.as_str()))
        .map(|required| Diagnostic {
            code: "capability_unavailable".to_owned(),
            severity: DiagnosticSeverity::Error,
            path: "$.capabilities.required".to_owned(),
            message: format!("required capability {} is unavailable", required.as_str()),
        })
        .collect())
}

#[derive(Default)]
struct SyntheticExecutionPort {
    runs: Mutex<BTreeMap<String, SyntheticRun>>,
}

struct SyntheticRun {
    runtime: StrictRuntime,
    executor: FixtureExecutor,
    definition_digest: String,
    cancelled: bool,
}

#[derive(Default)]
struct FixtureExecutor;

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
        let snapshot = snapshot_from_runtime(&run_id, definition_digest, &runtime, 1, false);
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
        };
        self.runs.lock().map_err(lock_error)?.insert(
            run_id,
            SyntheticRun {
                runtime,
                executor: FixtureExecutor,
                definition_digest: definition_digest.to_owned(),
                cancelled: false,
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
        let status = match context.request.kind {
            CommandKind::Pause => {
                run.runtime.pause().map_err(runtime_error)?;
                WorkflowRunStatus::Paused
            }
            CommandKind::Resume => {
                run.runtime.resume().map_err(runtime_error)?;
                WorkflowRunStatus::Running
            }
            CommandKind::Step => {
                run.runtime.step(&mut run.executor).map_err(runtime_error)?;
                management_status(run.runtime.status(), run.cancelled)
            }
            CommandKind::Cancel => {
                run.cancelled = true;
                WorkflowRunStatus::Cancelled
            }
        };
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
        )
        .with_status(status);
        Ok(CommandApplication {
            snapshot,
            outcome: super::contract::CommandOutcome::Applied,
            reason_code: context.request.kind.reason_code().to_owned(),
        })
    }
}

impl WorkflowReplayPort for SyntheticReplayPort {
    fn replay(
        &self,
        _request: &super::contract::ReplayRequest,
        snapshot: &RunSnapshot,
        events: &[RunEvent],
    ) -> Result<ReplayResult, ManagementError> {
        let mut expected = 1_u64;
        for event in events {
            if event.sequence != expected
                || event.workflow_run_id != snapshot.workflow_run_id
                || event.definition_digest != snapshot.definition_digest
            {
                return Ok(ReplayResult {
                    matched: false,
                    compared_events: expected.saturating_sub(1),
                    first_divergence: Some(super::contract::ReplayDivergence {
                        path: format!("/events/{expected}"),
                        code: "event_sequence_or_digest".to_owned(),
                    }),
                });
            }
            expected = expected.saturating_add(1);
        }
        Ok(ReplayResult {
            matched: !events.is_empty(),
            compared_events: events.len() as u64,
            first_divergence: None,
        })
    }
}

struct SyntheticReplayPort;

#[derive(Default)]
struct SyntheticCapabilityPort;

impl CapabilityPort for SyntheticCapabilityPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({
            "schema_version": "ascension.capabilities/v1",
            "producer": "sts2-harness.synthetic",
            "profile": "sts2-synthetic-v1",
            "capabilities": [
                "observe.fair-play.v1", "actions.catalog.v1", "actions.settlement.v1",
                "authority.generation-fence.v1", "actions.setup.v1", "actions.map.v1",
                "observe.map.v1", "actions.combat.v1", "actions.reward.v1",
                "actions.shop.v1", "actions.event.v1", "actions.rest.v1",
                "actions.selection.v1", "operations.reconcile.v1", "terminal.observation.v1",
                "analysis.combat.v1", "analysis.map.v1"
            ],
            "evidence_scope": "synthetic"
        }))
    }
}

fn snapshot_from_runtime(
    run_id: &str,
    definition_digest: &str,
    runtime: &StrictRuntime,
    revision: u64,
    cancelled: bool,
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
    }
}

trait StatusOverride {
    fn with_status(self, status: WorkflowRunStatus) -> Self;
}

impl StatusOverride for RunSnapshot {
    fn with_status(mut self, status: WorkflowRunStatus) -> Self {
        self.status = status;
        self
    }
}

fn management_status(status: RuntimeStatus, cancelled: bool) -> WorkflowRunStatus {
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

fn runtime_error(error: RuntimeFault) -> ManagementError {
    match error {
        RuntimeFault::BudgetExceeded => {
            ManagementError::budget("runtime_budget_exhausted", error.to_string())
        }
        RuntimeFault::UnknownEffect => ManagementError::unresolved(
            "unknown_effect",
            "synthetic runtime reported an unresolved effect",
        ),
        RuntimeFault::InvalidState => ManagementError::conflict("runtime_state", error.to_string()),
        _ => ManagementError::unavailable("runtime_failure", error.to_string()),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ManagementError {
    ManagementError::store("runtime_lock", "synthetic runtime lock is poisoned")
}
