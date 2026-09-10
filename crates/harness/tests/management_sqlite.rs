// SPDX-License-Identifier: MIT

use sts2_harness::management::{
    AuthContext, Budget, CleanupState, CommandAcceptance, CommandKind, CommandParameters,
    CommandRequest, Cursor, EVENT_SCHEMA_VERSION, EventClassification, EventPayload, EventType,
    GameOutcome, MANAGEMENT_SCHEMA_VERSION, ManagementReplayRequest, RUN_SCHEMA_VERSION, RunEvent,
    RunRequest, RunSnapshot, SqliteWorkflowStore, WorkflowRunStatus, WorkflowStore, digest_value,
    synthetic_sqlite_store,
};

fn snapshot(run_id: &str, digest: &str) -> RunSnapshot {
    RunSnapshot {
        schema_version: RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        definition_digest: digest.to_owned(),
        run_revision: 1,
        status: WorkflowRunStatus::Created,
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

fn event(run: &RunSnapshot) -> RunEvent {
    RunEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run.workflow_run_id.clone(),
        sequence: 1,
        run_revision: run.run_revision,
        event_type: EventType::RunStarted,
        definition_digest: run.definition_digest.clone(),
        node_execution_id: "management".to_owned(),
        payload: EventPayload {
            operation_id: None,
            classification: Some(EventClassification::Accepted),
            reason_code: "submitted".to_owned(),
        },
        integrity_digest: None,
    }
}

#[test]
fn sqlite_store_reopens_durable_run_and_event_state() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-management-sqlite-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("store.sqlite3");
    let store = SqliteWorkflowStore::open(&path)?;
    let digest = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let run = snapshot("run-sqlite", digest);
    store.create_run("request-sqlite", digest, run.clone(), vec![event(&run)])?;
    drop(store);

    let reopened = SqliteWorkflowStore::open(&path)?;
    assert_eq!(reopened.get_run("run-sqlite")?, Some(run));
    let page = reopened.events("run-sqlite", 0, 128)?;
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_type, EventType::RunStarted);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn sqlite_store_enables_durable_local_database_pragmas() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-management-pragmas-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("store.sqlite3");
    let store = SqliteWorkflowStore::open(&path)?;
    drop(store);

    let connection = rusqlite::Connection::open(&path)?;
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    let foreign_keys: i64 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(synchronous, 2);
    assert_eq!(foreign_keys, 1);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn sqlite_store_serializes_in_flight_commands_across_connections()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!(
        "sts2-management-command-lease-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("store.sqlite3");
    let first_store = SqliteWorkflowStore::open(&path)?;
    let digest = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let run = snapshot("run-command-lease", digest);
    first_store.create_run(
        "request-command-lease",
        digest,
        run.clone(),
        vec![event(&run)],
    )?;

    let first = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: "command-first".to_owned(),
        run_id: run.workflow_run_id.clone(),
        expected_revision: run.run_revision,
        actor_scope: "operator".to_owned(),
        kind: CommandKind::Pause,
        parameters: CommandParameters::default(),
    };
    let first_digest = digest_value(&serde_json::to_value(&first)?)?;
    assert!(matches!(
        first_store.accept_command(&first, &first_digest)?,
        CommandAcceptance::New { .. }
    ));

    let second_store = SqliteWorkflowStore::open(&path)?;
    let second = CommandRequest {
        command_id: "command-second".to_owned(),
        kind: CommandKind::Step,
        ..first.clone()
    };
    let second_digest = digest_value(&serde_json::to_value(&second)?)?;
    assert!(matches!(
        second_store.accept_command(&second, &second_digest)?,
        CommandAcceptance::Existing {
            response: None,
            application_in_flight: true,
            ..
        }
    ));
    assert_eq!(
        second_store
            .get_run(&run.workflow_run_id)?
            .map(|snapshot| snapshot.run_revision),
        Some(1)
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn persistent_synthetic_runtime_restores_after_service_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-management-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("runtime.sqlite3");
    let actor = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let definition: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-runtime".to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-runtime".to_owned(),
        profile: "synthetic".to_owned(),
    };
    let store = std::sync::Arc::new(SqliteWorkflowStore::open(&path)?);
    let service = synthetic_sqlite_store(std::sync::Arc::clone(&store));
    let admitted = service.submit_run(&actor, request)?;
    let command = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: "command-runtime".to_owned(),
        run_id: admitted.workflow_run_id.clone(),
        expected_revision: admitted.run_revision,
        actor_scope: "operator".to_owned(),
        kind: CommandKind::Step,
        parameters: CommandParameters::default(),
    };
    service.command(&actor, command.clone())?;
    drop(service);
    drop(store);

    let reopened = std::sync::Arc::new(SqliteWorkflowStore::open(&path)?);
    let restarted = synthetic_sqlite_store(reopened);
    let status = restarted.status(&actor, &admitted.workflow_run_id)?;
    let retry = CommandRequest {
        expected_revision: status.run.run_revision,
        command_id: "command-runtime-restart".to_owned(),
        ..command
    };
    let response = restarted.command(&actor, retry)?;
    assert_eq!(
        response.outcome,
        sts2_harness::management::CommandOutcome::Applied
    );
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn offline_replay_rejects_a_tampered_persisted_event() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-management-replay-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("replay.sqlite3");
    let actor = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let definition: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    let store = std::sync::Arc::new(SqliteWorkflowStore::open(&path)?);
    let service = synthetic_sqlite_store(std::sync::Arc::clone(&store));
    let run = service.submit_run(
        &actor,
        RunRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            request_id: "request-replay".to_owned(),
            definition: Some(definition),
            artifact_id: None,
            instance_id: "instance-replay".to_owned(),
            profile: "synthetic".to_owned(),
        },
    )?;
    let connection = rusqlite::Connection::open(&path)?;
    let bytes: Vec<u8> = connection.query_row(
        "SELECT event FROM management_events WHERE workflow_run_id = ?1 AND sequence = 1",
        [&run.workflow_run_id],
        |row| row.get(0),
    )?;
    let mut event: RunEvent = serde_json::from_slice(&bytes)?;
    event.payload.reason_code = "tampered-private-text".to_owned();
    connection.execute(
        "UPDATE management_events SET event = ?2
         WHERE workflow_run_id = ?1 AND sequence = 1",
        rusqlite::params![run.workflow_run_id, serde_json::to_vec(&event)?],
    )?;
    let error = match service.replay(
        &actor,
        ManagementReplayRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            run_id: run.workflow_run_id,
            offline: true,
        },
    ) {
        Ok(_) => return Err("tampered event unexpectedly replayed".into()),
        Err(error) => error,
    };
    assert_eq!(error.class, sts2_harness::management::ErrorClass::Replay);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
