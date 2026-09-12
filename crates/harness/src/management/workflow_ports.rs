// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::auth::AuthContext;
use super::contract::{
    Budget, CleanupState, CommandKind, ContextAssociationContext, ContextAvailability,
    ContextCaptureEvidence, ContextCaptureMode, ContextCaptureState, ContextInspectionCapabilities,
    Cursor, Diagnostic, DiagnosticSeverity, EventClassification, EventPayload, EventType,
    GameOutcome, RunEvent, RunRequest, RunSnapshot, WorkflowRunStatus, validate_identifier,
};
use super::service::{
    CapabilityPort, CommandApplication, CommandContext, ContextInspectionPort,
    ContextInspectionResult, DefinitionPort, DiffResult, InspectionResult, ManagementError,
    ReplayResult, RunAdmission, ValidationResult, WorkflowExecutionPort, WorkflowReplayPort,
};
use crate::workflow::{
    CompiledWorkflow, DecodeError, NodeExecutor, NodeOutcome, RuntimeContext, RuntimeFault,
    RuntimeStatus, StrictRuntime, TypedValue, WorkflowDefinition, decode_strict, semantic_diff,
    validate_definition,
};
use crate::{ContextBoundary, ControlAuthority, GateStatus};

/// Builds the authenticated synthetic service used by the CLI and process driver.
/// The adapter owns no gameplay authority and returns only deterministic fixture
/// outcomes through the same strict runtime boundary used by component tests.
pub fn synthetic_file_store(
    store: super::store::FileWorkflowStore,
) -> super::service::ManagementService {
    synthetic_store(Arc::new(store))
}

pub fn synthetic_store(
    store: Arc<dyn super::store::WorkflowStore>,
) -> super::service::ManagementService {
    super::service::ManagementService::new(store)
        .with_definition_port(Arc::new(SyntheticDefinitionPort))
        .with_execution_port(Arc::new(SyntheticExecutionPort::default()))
        .with_replay_port(Arc::new(SyntheticReplayPort))
        .with_capability_port(Arc::new(SyntheticCapabilityPort))
        .with_context_inspection_port(Arc::new(SyntheticContextInspectionPort))
}

pub fn synthetic_sqlite_store(
    store: Arc<super::store::SqliteWorkflowStore>,
) -> super::service::ManagementService {
    let service_store: Arc<dyn super::store::WorkflowStore> = store.clone();
    super::service::ManagementService::new(service_store)
        .with_authoring_store(store.clone())
        .with_definition_port(Arc::new(SyntheticDefinitionPort))
        .with_execution_port(Arc::new(PersistentSyntheticExecutionPort::new(store)))
        .with_replay_port(Arc::new(SyntheticReplayPort))
        .with_capability_port(Arc::new(SyntheticCapabilityPort))
        .with_context_inspection_port(Arc::new(SyntheticContextInspectionPort))
}

struct SyntheticContextInspectionPort;

impl ContextInspectionPort for SyntheticContextInspectionPort {
    fn inspect(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextInspectionResult, ManagementError> {
        Ok(ContextInspectionResult {
            context: ContextAssociationContext {
                availability: ContextAvailability::Unavailable,
                context_ref: None,
                run_id: None,
                episode_id: None,
                agent_id: None,
                snapshot_id: None,
                approved_revision_id: None,
                plan_epoch: None,
                reason_code: Some("synthetic_context_adapter_unavailable".to_owned()),
            },
            capture: ContextCaptureEvidence {
                mode: ContextCaptureMode::Unavailable,
                state: ContextCaptureState::Unavailable,
                attempt_id: None,
                reason_code: Some("synthetic_context_adapter_unavailable".to_owned()),
            },
            capabilities: ContextInspectionCapabilities {
                inspect_metadata: true,
                ..ContextInspectionCapabilities::default()
            },
        })
    }
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
        diagnostics.extend(context_reference_diagnostics(&parsed, capabilities)?);
        Ok(ValidationResult {
            definition_digest: digest,
            compiler: crate::workflow::WORKFLOW_COMPILER_ID.to_owned(),
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

/// Validates context references only when the owner has disclosed its bounded
/// binding catalog. An absent catalog preserves compatibility with owners that
/// have not yet installed the integration adapter; a malformed disclosed
/// catalog fails closed rather than silently accepting an unresolvable ref.
fn context_reference_diagnostics(
    definition: &WorkflowDefinition,
    manifest: &Value,
) -> Result<Vec<Diagnostic>, ManagementError> {
    let Some(entries) = manifest.get("context_bindings") else {
        return Ok(Vec::new());
    };
    let entries = entries.as_array().ok_or_else(|| {
        ManagementError::invalid(
            "context_binding_manifest",
            "context_bindings must be an array when disclosed",
        )
    })?;
    let mut bindings = BTreeMap::<String, BTreeSet<String>>::new();
    for entry in entries {
        let object = entry.as_object().ok_or_else(|| {
            ManagementError::invalid(
                "context_binding_manifest",
                "each context binding must be an object",
            )
        })?;
        if object.len() != 2 {
            return Err(ManagementError::invalid(
                "context_binding_manifest",
                "each context binding must contain context_ref and node_kinds only",
            ));
        }
        let context_ref = object
            .get("context_ref")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_binding_manifest",
                    "each context binding needs a context_ref",
                )
            })?;
        validate_identifier("context_ref", context_ref)?;
        let kinds = object
            .get("node_kinds")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_binding_manifest",
                    "each context binding needs node_kinds",
                )
            })?;
        if kinds.is_empty() {
            return Err(ManagementError::invalid(
                "context_binding_manifest",
                "each context binding needs at least one supported node kind",
            ));
        }
        let mut supported = BTreeSet::new();
        for kind in kinds {
            let kind = kind.as_str().ok_or_else(|| {
                ManagementError::invalid(
                    "context_binding_manifest",
                    "context binding node kinds must be strings",
                )
            })?;
            if !matches!(kind, "analyze" | "decide") {
                return Err(ManagementError::invalid(
                    "context_binding_manifest",
                    "context binding node kind is unsupported",
                ));
            }
            supported.insert(kind.to_owned());
        }
        if bindings.insert(context_ref.to_owned(), supported).is_some() {
            return Err(ManagementError::invalid(
                "context_binding_manifest",
                "context binding references must be unique",
            ));
        }
    }

    let mut diagnostics = Vec::new();
    for graph in &definition.graphs {
        for node in &graph.nodes {
            let (kind, context_ref) = match node {
                crate::workflow::NodeDefinition::Analyze { config, .. } => {
                    ("analyze", config.context_ref.as_str())
                }
                crate::workflow::NodeDefinition::Decide { config, .. } => {
                    ("decide", config.context_ref.as_str())
                }
                _ => continue,
            };
            let supported = bindings.get(context_ref);
            let code = if supported.is_none() {
                "context_ref_unresolved"
            } else if !supported.is_some_and(|kinds| kinds.contains(kind)) {
                "context_ref_incompatible"
            } else {
                continue;
            };
            diagnostics.push(Diagnostic {
                code: code.to_owned(),
                severity: DiagnosticSeverity::Error,
                path: format!(
                    "$.graphs.{}.nodes.{}.config.context_ref",
                    graph.id,
                    node.id()
                ),
                message: format!(
                    "context reference {context_ref} is not available for {kind} nodes"
                ),
            });
        }
    }
    Ok(diagnostics)
}

#[derive(Default)]
struct SyntheticExecutionPort {
    runs: Mutex<BTreeMap<String, SyntheticRun>>,
}

struct SyntheticRun {
    runtime: StrictRuntime,
    executor: FixtureExecutor,
    definition: Value,
    definition_digest: String,
    cancelled: bool,
    /// Present only for the in-memory synthetic execution path. It is the same
    /// Harness control authority that fences its pause/resume commands; the
    /// Context Console reducer is never consulted here.
    control: Option<ControlAuthority>,
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
        )
        .with_status(status);
        Ok(CommandApplication {
            snapshot,
            outcome: super::contract::CommandOutcome::Applied,
            reason_code: context.request.kind.reason_code().to_owned(),
        })
    }
}

/// Synthetic execution with a durable definition and runtime checkpoint. It exercises the
/// management process restart boundary while keeping gameplay effects explicitly unavailable.
struct PersistentSyntheticExecutionPort {
    store: Arc<super::store::SqliteWorkflowStore>,
    runs: Mutex<BTreeMap<String, SyntheticRun>>,
}

impl PersistentSyntheticExecutionPort {
    fn new(store: Arc<super::store::SqliteWorkflowStore>) -> Self {
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
        let snapshot = snapshot_from_runtime(&run_id, definition_digest, &run.runtime, 1, false);
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
        )
        .with_status(status);
        Ok(CommandApplication {
            snapshot,
            outcome: super::contract::CommandOutcome::Applied,
            reason_code: context.request.kind.reason_code().to_owned(),
        })
    }
}

fn apply_controlled_runtime_command(
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

fn apply_runtime_command(
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

fn synthetic_control_boundary(run_id: &str) -> ContextBoundary {
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

fn control_error(code: String) -> ManagementError {
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

fn admission_event(run_id: &str, definition_digest: &str) -> RunEvent {
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
                || !event.integrity_valid()
            {
                return Ok(ReplayResult {
                    matched: false,
                    compared_events: expected.saturating_sub(1),
                    first_divergence: Some(super::contract::ReplayDivergence {
                        path: format!("/events/{expected}"),
                        code: if event.integrity_valid() {
                            "event_sequence_or_digest".to_owned()
                        } else {
                            "event_integrity".to_owned()
                        },
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
            "context_bindings": [
                {"context_ref": "context.synthetic.v1", "node_kinds": ["analyze", "decide"]},
                {"context_ref": "sts2.setup.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.map.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.combat.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.reward.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.shop.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.event.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.rest.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.selection.context.v1", "node_kinds": ["decide"]},
                {"context_ref": "sts2.campaign.context.v1", "node_kinds": ["decide"]}
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

fn runtime_store_error(error: super::store::StoreError) -> ManagementError {
    ManagementError::store(error.code, error.message)
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ManagementError {
    ManagementError::store("runtime_lock", "synthetic runtime lock is poisoned")
}
