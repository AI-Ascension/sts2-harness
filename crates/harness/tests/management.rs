// SPDX-License-Identifier: MIT

use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, Budget, CleanupState, CommandApplication, CommandKind, CommandParameters,
    CommandRequest, Cursor, DefinitionPort, DiffRequest, DiffResult, EVENT_SCHEMA_VERSION,
    ErrorClass, EventClassification, EventPayload, EventType, FileWorkflowStore, GameOutcome,
    InspectRequest, InspectionResult, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementError,
    ManagementReplayRequest, ManagementServer, ManagementService, MemoryWorkflowStore,
    OutputFormat, RUN_SCHEMA_VERSION, RecoveryAdmission, ReplayResult, RunAdmission, RunEvent,
    RunRequest, RunSnapshot, ServerConfig, StaticAuthenticator, ValidateRequest, ValidationResult,
    WorkflowExecutionPort, WorkflowReplayPort, WorkflowRunStatus, WorkflowStore, decode_strict,
    digest_value,
};

struct DefinitionDouble;

impl DefinitionPort for DefinitionDouble {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Ok(ValidationResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            diagnostics: Vec::new(),
        })
    }

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        Ok(InspectionResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            workflow_id: Some("fixture".to_owned()),
            workflow_version: Some("1.0.0".to_owned()),
            required_capabilities: Vec::new(),
            graph_count: 1,
            node_count: 1,
        })
    }

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        Ok(DiffResult {
            old_definition_digest: digest_value(old_definition).map_err(ManagementError::from)?,
            new_definition_digest: digest_value(new_definition).map_err(ManagementError::from)?,
            semantic_change: old_definition != new_definition,
            changed_paths: if old_definition == new_definition {
                Vec::new()
            } else {
                vec!["/".to_owned()]
            },
        })
    }
}

struct ExecutionDouble {
    submissions: AtomicUsize,
    commands: AtomicUsize,
}

impl ExecutionDouble {
    fn new() -> Self {
        Self {
            submissions: AtomicUsize::new(0),
            commands: AtomicUsize::new(0),
        }
    }
}

impl WorkflowExecutionPort for ExecutionDouble {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        let snapshot = snapshot("run-1", definition_digest, 1, WorkflowRunStatus::Created);
        Ok(RunAdmission {
            initial_events: vec![event(&snapshot, 1, EventType::RunStarted, "submitted")],
            snapshot,
        })
    }

    fn apply_command(
        &self,
        context: sts2_harness::management::CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        self.commands.fetch_add(1, Ordering::SeqCst);
        let status = match context.request.kind {
            CommandKind::Pause => WorkflowRunStatus::Paused,
            CommandKind::Resume | CommandKind::Step => WorkflowRunStatus::Running,
            CommandKind::Cancel => WorkflowRunStatus::Cancelling,
        };
        let mut next = context.snapshot;
        next.run_revision = next
            .run_revision
            .checked_add(1)
            .ok_or_else(|| ManagementError::conflict("overflow", "revision overflow"))?;
        next.status = status;
        Ok(CommandApplication {
            snapshot: next,
            outcome: sts2_harness::management::CommandOutcome::Applied,
            reason_code: context.request.kind.reason_code().to_owned(),
        })
    }
}

struct ReplayDouble {
    calls: AtomicUsize,
}

impl ReplayDouble {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl WorkflowReplayPort for ReplayDouble {
    fn replay(
        &self,
        _request: &ManagementReplayRequest,
        _snapshot: &RunSnapshot,
        events: &[RunEvent],
    ) -> Result<ReplayResult, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ReplayResult {
            matched: true,
            compared_events: events.len() as u64,
            first_divergence: None,
        })
    }
}

fn actor() -> Result<AuthContext, sts2_harness::management::AuthError> {
    AuthContext::new("operator", ["workflow:*".to_owned()])
}

fn snapshot(
    run_id: &str,
    definition_digest: &str,
    revision: u64,
    status: WorkflowRunStatus,
) -> RunSnapshot {
    RunSnapshot {
        schema_version: RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        definition_digest: definition_digest.to_owned(),
        run_revision: revision,
        status,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: "graph".to_owned(),
            node_id: "node".to_owned(),
            node_execution_id: "node-exec".to_owned(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
    }
}

fn event(snapshot: &RunSnapshot, sequence: u64, event_type: EventType, reason: &str) -> RunEvent {
    RunEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: snapshot.workflow_run_id.clone(),
        sequence,
        run_revision: snapshot.run_revision,
        event_type,
        definition_digest: snapshot.definition_digest.clone(),
        node_execution_id: "management".to_owned(),
        payload: EventPayload {
            operation_id: None,
            classification: Some(EventClassification::Accepted),
            reason_code: reason.to_owned(),
        },
        integrity_digest: None,
    }
}

fn service_with_doubles(
    execution: Arc<ExecutionDouble>,
    replay: Arc<ReplayDouble>,
) -> ManagementService {
    ManagementService::in_memory()
        .with_definition_port(Arc::new(DefinitionDouble))
        .with_execution_port(execution)
        .with_replay_port(replay)
}

fn run_request() -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-1".to_owned(),
        definition: Some(json!({"workflow": "fixture"})),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "synthetic".to_owned(),
    }
}

#[test]
fn strict_management_json_rejects_duplicate_and_unknown_fields() {
    let duplicate = br#"{"schema_version":"ascension.management/v1","definition":{"x":1,"x":2},"capabilities":{}}"#;
    assert!(decode_strict::<ValidateRequest>(duplicate).is_err());

    let unknown = br#"{"schema_version":"ascension.management/v1","definition":{},"capabilities":{},"extra":true}"#;
    assert!(decode_strict::<ValidateRequest>(unknown).is_err());
}

#[test]
fn command_acceptance_is_idempotent_and_revision_bound() -> Result<(), Box<dyn std::error::Error>> {
    let execution = Arc::new(ExecutionDouble::new());
    let replay = Arc::new(ReplayDouble::new());
    let service = service_with_doubles(Arc::clone(&execution), replay);
    let actor = actor()?;

    let first = service.submit_run(&actor, run_request())?;
    let second = service.submit_run(&actor, run_request())?;
    assert_eq!(first.workflow_run_id, second.workflow_run_id);
    assert_eq!(execution.submissions.load(Ordering::SeqCst), 1);
    assert_eq!(
        service
            .status(&actor, &first.workflow_run_id)?
            .recovery_admission,
        RecoveryAdmission::NoPendingEffects
    );

    let request = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: "command-1".to_owned(),
        run_id: first.workflow_run_id.clone(),
        expected_revision: 1,
        actor_scope: "operator".to_owned(),
        kind: CommandKind::Pause,
        parameters: CommandParameters::default(),
    };
    let applied = service.command(&actor, request.clone())?;
    let duplicate = service.command(&actor, request)?;
    assert_eq!(applied, duplicate);
    assert_eq!(execution.commands.load(Ordering::SeqCst), 1);
    assert_eq!(
        service
            .status(&actor, &first.workflow_run_id)?
            .recovery_admission,
        RecoveryAdmission::SafelyResumable {
            capability: "synthetic.workflow.resume.v1".to_owned()
        }
    );

    let stale = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: "command-2".to_owned(),
        run_id: first.workflow_run_id.clone(),
        expected_revision: 1,
        actor_scope: "operator".to_owned(),
        kind: CommandKind::Resume,
        parameters: CommandParameters::default(),
    };
    let error = match service.command(&actor, stale) {
        Ok(_) => return Err("stale command unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert_eq!(error.class, ErrorClass::Conflict);

    let page = service.events(&actor, &first.workflow_run_id, 0, 128)?;
    assert_eq!(page.events.len(), 3);
    assert_eq!(page.events[1].event_type, EventType::CommandRequested);
    assert_eq!(page.events[2].event_type, EventType::CommandApplied);
    Ok(())
}

#[test]
fn read_only_paths_use_injected_ports_without_execution() -> Result<(), Box<dyn std::error::Error>>
{
    let execution = Arc::new(ExecutionDouble::new());
    let replay = Arc::new(ReplayDouble::new());
    let service = service_with_doubles(Arc::clone(&execution), Arc::clone(&replay));
    let actor = actor()?;
    let definition = json!({"workflow": "fixture"});

    let validation = service.validate(
        &actor,
        ValidateRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            definition: definition.clone(),
            capabilities: json!({}),
        },
    )?;
    assert!(validation.valid);
    service.inspect(
        &actor,
        InspectRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            definition: definition.clone(),
            format: OutputFormat::Json,
        },
    )?;
    service.diff(
        &actor,
        DiffRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            old_definition: definition.clone(),
            new_definition: definition,
            format: OutputFormat::Json,
        },
    )?;
    assert_eq!(execution.submissions.load(Ordering::SeqCst), 0);
    assert_eq!(execution.commands.load(Ordering::SeqCst), 0);
    assert_eq!(replay.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn loopback_server_requires_auth_and_enforces_read_only_route_bounds()
-> Result<(), Box<dyn std::error::Error>> {
    let service = Arc::new(ManagementService::in_memory());
    let auth = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let authenticator = Arc::new(StaticAuthenticator::single("secret", auth)?);
    let config = ServerConfig::new("127.0.0.1:0".parse::<SocketAddr>()?, authenticator)?;
    let server = ManagementServer::start(config, service)?;
    let address = server.address();

    let client = ManagementClient::new(address, "secret")?;
    let health = client.request_json("GET", "/v1/health", None)?;
    assert_eq!(health.status, 200);

    let unauthorized = raw_request(
        address,
        "GET /v1/capabilities HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n",
    )?;
    assert_eq!(unauthorized.status, 401);

    let origin = raw_request(
        address,
        "GET /v1/health HTTP/1.1\r\nHost: local\r\nOrigin: https://example.test\r\nConnection: close\r\n\r\n",
    )?;
    assert_eq!(origin.status, 403);
    server.shutdown()?;
    Ok(())
}

#[test]
fn file_store_reopens_durable_run_state() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("sts2-management-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("store.json");
    let store = FileWorkflowStore::open(&path)?;
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let run = snapshot("run-file", digest, 1, WorkflowRunStatus::Created);
    store.create_run(
        "request-file",
        digest,
        run.clone(),
        vec![event(&run, 1, EventType::RunStarted, "submitted")],
    )?;
    drop(store);
    let reopened = FileWorkflowStore::open(&path)?;
    assert_eq!(reopened.get_run("run-file")?, Some(run));
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn redacted_export_removes_untrusted_operation_and_cursor_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let store = MemoryWorkflowStore::new();
    let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut run = snapshot("run-private", digest, 1, WorkflowRunStatus::Running);
    run.cursor.graph_id = "sentinel-secret-graph".to_owned();
    run.cursor.node_id = "sentinel-secret-node".to_owned();
    run.cursor.node_execution_id = "sentinel-secret-execution".to_owned();
    let mut initial = event(
        &run,
        1,
        EventType::OperationUnknown,
        "sentinel-secret-reason",
    );
    initial.node_execution_id = "sentinel-secret-event".to_owned();
    initial.payload.operation_id = Some("sentinel-secret-operation".to_owned());
    initial.payload.reason_code = "sentinel-secret-reason".to_owned();
    store.create_run("request-private", digest, run, vec![initial])?;

    let exported = store.export("run-private", true)?;
    let encoded = serde_json::to_string(&exported)?;
    assert!(!encoded.contains("sentinel-secret"));
    assert_eq!(exported.run.cursor.graph_id, "[redacted]");
    assert_eq!(exported.run.pending_operation, None);
    assert_eq!(exported.events[0].payload.operation_id, None);
    assert_eq!(exported.events[0].payload.reason_code, "event");
    assert_eq!(exported.events[0].node_execution_id, "[redacted]");
    Ok(())
}

fn raw_request(
    address: SocketAddr,
    request: &str,
) -> Result<RawResponse, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    std::io::Write::write_all(&mut stream, request.as_bytes())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut stream, &mut bytes)?;
    let text = String::from_utf8(bytes)?;
    let status = text
        .split_ascii_whitespace()
        .nth(1)
        .ok_or("missing response status")?
        .parse::<u16>()?;
    Ok(RawResponse { status })
}

struct RawResponse {
    status: u16,
}
// SPDX-License-Identifier: MIT
