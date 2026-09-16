// SPDX-License-Identifier: MIT

//! Run snapshots must stay inside the closed Studio contract, while the
//! harness must still refuse a serving port whose declared execution mode
//! contradicts the admitted target.
#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, Budget, CleanupState, CommandApplication, CommandContext, Cursor, ExecutionMode,
    GameOutcome, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementError, ManagementServer,
    RUN_SCHEMA_VERSION, RunAdmission, RunRequest, RunSnapshot, ServerConfig, SqliteWorkflowStore,
    StaticAuthenticator, WorkflowExecutionPort, WorkflowRunStatus, synthetic_sqlite_store,
};

/// The pinned consumer `AI-Ascension/ascension-workflow-studio`
/// `@31c5e5f407ab17fb0363374dd1d926c25e6350ea` (`packages/contracts/src/
/// index.ts`, `RunSnapshotSchema`) parses run snapshots as
/// `z.object({..}).strict()` with exactly these keys, so any additional key
/// fails consumer parsing. This list is the authority for what the wire may
/// carry; extending it requires version/capability negotiation plus a
/// coordinated consumer pin update (issue #94).
const PINNED_RUN_SNAPSHOT_KEYS: [&str; 11] = [
    "schema_version",
    "workflow_run_id",
    "definition_digest",
    "run_revision",
    "status",
    "game_outcome",
    "cursor",
    "pending_operation",
    "budget",
    "cleanup",
    "admission",
];

fn synthetic_request(request_id: &str) -> Result<RunRequest, Box<dyn std::error::Error>> {
    let definition: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    Ok(RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "sts2-synthetic-1".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    })
}

fn operator() -> Result<AuthContext, Box<dyn std::error::Error>> {
    Ok(AuthContext::new("operator", ["workflow:*".to_owned()])?)
}

/// The synthetic composition served by `management/cli.rs` `serve` must not
/// leak the internal execution-mode report into the closed run-snapshot wire
/// contract that the pinned Studio consumer parses.
#[test]
fn served_run_snapshots_never_expose_the_internal_execution_mode()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("sts2-run-mode-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let store = SqliteWorkflowStore::open(directory.join("store.sqlite"))?;
    let service = Arc::new(synthetic_sqlite_store(Arc::new(store)));
    let authenticator =
        StaticAuthenticator::new().with_credential("operator-token", operator()?)?;
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        Arc::new(authenticator),
    )?;
    let server = ManagementServer::start(config, service)?;
    let client = ManagementClient::new(server.address(), "operator-token")?;

    let request = synthetic_request("request-run-mode")?;
    let response = client.request_json(
        "POST",
        "/v1/workflow-runs",
        Some(&serde_json::to_vec(&request)?),
    )?;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let submitted: serde_json::Value = serde_json::from_slice(&response.body)?;
    let run_id = submitted["workflow_run_id"].as_str().expect("run id");
    let path = format!("/v1/workflow-runs/{run_id}");
    let status_response = client.request_json("GET", &path, None)?;
    let body = String::from_utf8_lossy(&status_response.body);
    assert!(
        !body.contains("execution_mode"),
        "an unnegotiated execution_mode key reached the wire: {body}"
    );
    let status: serde_json::Value = serde_json::from_slice(&status_response.body)?;
    let run = status["run"].as_object().expect("run snapshot object");
    for key in run.keys() {
        assert!(
            PINNED_RUN_SNAPSHOT_KEYS.contains(&key.as_str()),
            "run snapshot key `{key}` is absent from the pinned closed schema"
        );
    }

    server.shutdown()?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

/// Mutant: a port that serves this synthetic composition but declares live
/// execution. The harness must refuse it pre-dispatch with
/// `port_execution_mode_mismatch` instead of persisting or returning the run.
struct WrongModeExecutionPort;

impl WorkflowExecutionPort for WrongModeExecutionPort {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        Ok(RunAdmission {
            snapshot: RunSnapshot {
                schema_version: RUN_SCHEMA_VERSION.to_owned(),
                workflow_run_id: "run-mutant".to_owned(),
                definition_digest: "a".repeat(64),
                run_revision: 1,
                status: WorkflowRunStatus::Running,
                game_outcome: GameOutcome::NotTerminal,
                cursor: Cursor {
                    graph_id: "graph".to_owned(),
                    node_id: "node".to_owned(),
                    node_execution_id: "node-exec".to_owned(),
                },
                pending_operation: None,
                budget: Budget::default(),
                cleanup: CleanupState::NotStarted,
                admission: None,
                execution_mode: Some(ExecutionMode::Live),
            },
            initial_events: Vec::new(),
        })
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::invalid(
            "mutant_port_unavailable",
            "the mutant port never applies commands",
        ))
    }
}

#[test]
fn a_port_reporting_a_mode_that_contradicts_the_admitted_target_is_refused()
-> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-run-mode-mutant-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let store = SqliteWorkflowStore::open(directory.join("store.sqlite"))?;
    let service = synthetic_sqlite_store(Arc::new(store))
        .with_execution_port(Arc::new(WrongModeExecutionPort));

    let error = service
        .submit_run(&operator()?, synthetic_request("request-run-mode-mutant")?)
        .expect_err("a live port report must not satisfy a synthetic admission");
    assert_eq!(error.code, "port_execution_mode_mismatch");

    std::fs::remove_dir_all(directory)?;
    Ok(())
}
