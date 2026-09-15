// SPDX-License-Identifier: MIT

//! Deterministic component evidence: SQLite close/reopen and caller retries.
//! No browser, management transport, provider, gateway, or native game is involved.

#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use sts2_harness::management::{
    CommandKind, CommandOutcome, CommandRequest, CommandResponse, EventPage, LiveWorkflowOptions,
    LiveWorkflowSessionFactory, ManagementService, PendingOperationState, RecoveryAdmission,
    RunSnapshot, SqliteWorkflowStore, WorkflowRunStatus, WorkflowStore,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

#[test]
fn unknown_action_response_replay_survives_sqlite_reopen_without_redispatch() {
    replay_after_reopen(FakeFactory::new(true), Some(PendingOperationState::Unknown));
}

#[test]
fn lost_dispatch_response_replay_preserves_intent_across_sqlite_reopen() {
    replay_after_reopen(
        FakeFactory::dispatch_error(),
        Some(PendingOperationState::Intent),
    );
}

#[test]
fn settled_action_response_replay_survives_sqlite_reopen_without_redispatch() {
    replay_after_reopen(FakeFactory::new(false), None);
}

fn replay_after_reopen(factory: FakeFactory, pending: Option<PendingOperationState>) {
    let database = Database::new();
    let factory = Arc::new(factory);
    let store = Arc::new(SqliteWorkflowStore::open(&database.path).expect("SQLite"));
    let service = service_for(Arc::clone(&store), &factory);
    let run_id = start_action(&service);
    let action = command(&run_id, "execute-once", 3, CommandKind::Step);
    // The oracle retains the server response; the simulated caller loses it.
    let original = service.command(&actor(), action.clone()).expect("action");
    let evidence = Evidence::capture(&service, &run_id, original);
    evidence.assert_action_result(pending.clone());
    let calls = factory.entries();
    let mut expected_calls = vec!["launch", "observe", "legal_actions", "decide", "dispatch"];
    if pending.is_none() {
        expected_calls.push("wait");
    }
    assert_eq!(calls, expected_calls);
    assert_replay(&service, &action, &evidence);
    assert_conflicting_command(&service, &action);
    assert_conflicting_submission(&service);
    evidence.assert_durable_state(&service, &run_id);
    assert_eq!(factory.entries(), calls);

    drop(service);
    // Prove no service/store Arc can keep the old SQLite connection alive.
    assert_eq!(Arc::strong_count(&store), 1);
    drop(store);
    let restarted_store = Arc::new(SqliteWorkflowStore::open(&database.path).expect("reopen"));
    let restarted_factory = Arc::new(FakeFactory::new(false));
    let restarted = service_for(Arc::clone(&restarted_store), &restarted_factory);
    assert_replay(&restarted, &action, &evidence);
    assert_conflicting_command(&restarted, &action);
    assert_conflicting_submission(&restarted);
    evidence.assert_durable_state(&restarted, &run_id);
    assert_restart_blocked(&restarted, &run_id, &evidence);
    assert_eq!(factory.entries(), calls);
    assert!(restarted_factory.entries().is_empty());
    assert!(restarted_factory.launches().is_empty());
    drop(restarted);
    assert_eq!(Arc::strong_count(&restarted_store), 1);
    drop(restarted_store);
}

fn service_for(store: Arc<SqliteWorkflowStore>, factory: &Arc<FakeFactory>) -> ManagementService {
    live_service(
        store as Arc<dyn WorkflowStore>,
        Arc::clone(factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("live component service")
}

fn start_action(service: &ManagementService) -> String {
    let submission = service
        .submit_run(&actor(), request("submit-once", definition(false)))
        .expect("submit");
    let run_id = submission.workflow_run_id;
    for (id, revision) in [("observe-once", 1), ("decide-once", 2)] {
        service
            .command(&actor(), command(&run_id, id, revision, CommandKind::Step))
            .expect("prepare action");
    }
    run_id
}

fn assert_replay(service: &ManagementService, request: &CommandRequest, evidence: &Evidence) {
    let replay = service
        .command(&actor(), request.clone())
        .expect("exact request replay");
    assert_eq!(replay, evidence.response);
    evidence.assert_durable_state(service, &request.run_id);
}

fn assert_conflicting_command(service: &ManagementService, request: &CommandRequest) {
    let mut conflict = request.clone();
    conflict.kind = CommandKind::Cancel;
    let error = service
        .command(&actor(), conflict)
        .expect_err("same command ID with changed payload");
    assert_eq!(error.code, "command_conflict");
}

fn assert_conflicting_submission(service: &ManagementService) {
    let mut changed = definition(false);
    changed["limits"]["max_steps"] = serde_json::json!(31);
    let error = service
        .submit_run(&actor(), request("submit-once", changed))
        .expect_err("same submission request ID with a changed graph");
    assert_eq!(error.code, "submission_conflict");
}

fn assert_restart_blocked(service: &ManagementService, run_id: &str, evidence: &Evidence) {
    let snapshot = &evidence.snapshot;
    let recovery = service.status(&actor(), run_id).expect("status");
    assert_eq!(
        recovery.recovery_admission,
        if snapshot.pending_operation.is_some() {
            RecoveryAdmission::Reconciling
        } else {
            RecoveryAdmission::NeedsOperator
        }
    );
    let error = service
        .command(
            &actor(),
            command(
                run_id,
                "continue-after-restart",
                snapshot.run_revision,
                CommandKind::Step,
            ),
        )
        .expect_err("a missing live session cannot advance");
    assert_eq!(error.code, "live_runtime_after_restart");
    assert_eq!(
        service.status(&actor(), run_id).expect("status").run,
        *snapshot
    );
    // A fresh command is durably requested before the unavailable session rejects
    // it. It must not append another intent or claim an applied operation.
    let events = service.events(&actor(), run_id, 0, 128).expect("events");
    assert_eq!(events.events.len(), evidence.events.events.len() + 1);
    assert_eq!(
        &events.events[..evidence.events.events.len()],
        &evidence.events.events
    );
    let last = events.events.last().expect("requested command");
    assert_eq!(
        last.event_type,
        sts2_harness::management::EventType::CommandRequested
    );
    assert_eq!(last.run_revision, snapshot.run_revision);
    assert_eq!(
        Some(last.sequence),
        evidence.events.newest_sequence.map(|value| value + 1)
    );
}

struct Evidence {
    response: CommandResponse,
    snapshot: RunSnapshot,
    events: EventPage,
}

impl Evidence {
    fn capture(service: &ManagementService, run_id: &str, response: CommandResponse) -> Self {
        Self {
            response,
            snapshot: service.status(&actor(), run_id).expect("status").run,
            events: service.events(&actor(), run_id, 0, 128).expect("events"),
        }
    }

    fn assert_action_result(&self, pending: Option<PendingOperationState>) {
        assert_eq!(self.response.run_revision, 4);
        if let Some(classification) = pending {
            assert_eq!(self.response.outcome, CommandOutcome::Pending);
            assert_eq!(self.snapshot.status, WorkflowRunStatus::NeedsOperator);
            let operation = self.snapshot.pending_operation.as_ref().expect("operation");
            assert_eq!(operation.classification, classification);
            assert!(!operation.operation_id.is_empty());
            assert_eq!(operation.instance_id, "instance-1");
            assert_eq!(operation.original_generation, 0);
            assert_eq!(operation.payload_digest.len(), 64);
            assert!(self.events.events.iter().any(|event| {
                event.event_type == sts2_harness::management::EventType::OperationIntent
                    && event.payload.operation_id.as_ref() == Some(&operation.operation_id)
            }));
        } else {
            assert_eq!(self.response.outcome, CommandOutcome::Applied);
            assert_eq!(self.snapshot.status, WorkflowRunStatus::Running);
            assert!(self.snapshot.pending_operation.is_none());
            let applied = self.events.events.last().expect("applied action");
            assert_eq!(Some(applied.sequence), self.response.sequence);
            assert_eq!(
                applied.event_type,
                sts2_harness::management::EventType::CommandApplied
            );
            assert_eq!(
                applied.payload.classification,
                Some(sts2_harness::management::EventClassification::Settled)
            );
        }
    }

    fn assert_durable_state(&self, service: &ManagementService, run_id: &str) {
        assert_eq!(
            service.status(&actor(), run_id).expect("status").run,
            self.snapshot
        );
        assert_eq!(
            service.events(&actor(), run_id, 0, 128).expect("events"),
            self.events
        );
    }
}

struct Database {
    directory: PathBuf,
    path: PathBuf,
}

impl Database {
    fn new() -> Self {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/live-workflow-replay")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).expect("test database directory");
        let path = directory.join("workflow.sqlite3");
        Self { directory, path }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
