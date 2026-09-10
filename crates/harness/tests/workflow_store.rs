// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use sts2_harness::{
    ExecutionStore, ExecutionStoreConfig, ExecutionStoreError, GameOperationId, InvocationOutcome,
    InvocationState, RECOVERY_CONTRACT_VERSION, RunStatus, WorkflowCommandId, WorkflowDefinition,
    WorkflowDefinitionId, WorkflowEpisodeId, WorkflowEvent, WorkflowEventId, WorkflowEventPayload,
    WorkflowInvocation, WorkflowInvocationId, WorkflowPlan, WorkflowPlanId, WorkflowRunId,
    WorkflowRunStart,
};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

fn database_path(label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the epoch")
        .as_nanos();
    let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sts2-workflow-store-{label}-{}-{timestamp}-{sequence}.sqlite3",
        std::process::id()
    ))
}

fn remove_database(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = PathBuf::from(format!("{}{}", path.display(), suffix));
        let _ = fs::remove_file(candidate);
    }
}

fn setup_store(path: &Path) -> (ExecutionStore, WorkflowRunStart) {
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(path)).expect("store opens");
    let workflow = WorkflowDefinition::new(
        WorkflowDefinitionId::new("workflow-1").expect("workflow id is valid"),
        br#"{"nodes":["start"]}"#.to_vec(),
    )
    .expect("definition is valid");
    let plan = WorkflowPlan::new(
        WorkflowPlanId::new("plan-1").expect("plan id is valid"),
        workflow.id.clone(),
        br#"{"entry":"start"}"#.to_vec(),
    )
    .expect("plan is valid");
    store
        .register_workflow_definition(&workflow)
        .expect("definition is admitted");
    store
        .register_workflow_plan(&plan)
        .expect("plan is admitted");
    let start = WorkflowRunStart::new(
        WorkflowRunId::new("run-1").expect("run id is valid"),
        workflow.id,
        plan.id,
        WorkflowEpisodeId::new("episode-1").expect("episode id is valid"),
    )
    .expect("run is valid");
    store
        .start_workflow_run(
            &start,
            WorkflowEventId::new("event-1").expect("event id is valid"),
        )
        .expect("run starts");
    (store, start)
}

#[test]
fn typed_namespaces_and_immutable_definitions_are_enforced() {
    let path = database_path("identity");
    let (mut store, start) = setup_store(&path);
    let changed = WorkflowDefinition::new(
        WorkflowDefinitionId::new("workflow-1").expect("workflow id is valid"),
        br#"{"nodes":["changed"]}"#.to_vec(),
    )
    .expect("definition is valid");
    assert_eq!(
        store.register_workflow_definition(&changed),
        Err(ExecutionStoreError::Conflict)
    );
    assert_ne!(WorkflowDefinitionId::NAMESPACE, GameOperationId::NAMESPACE);
    assert!(WorkflowDefinitionId::new("bad id").is_err());
    let mut invalid_start = start.clone();
    invalid_start.initial_projection.cursor = u64::MAX;
    assert_eq!(
        invalid_start.validate(),
        Err(ExecutionStoreError::InvalidWorkflowProjection)
    );
    drop(store);
    remove_database(&path);
}

#[test]
fn journal_replay_and_compare_and_swap_survive_reopen() {
    let path = database_path("replay");
    let (mut store, start) = setup_store(&path);
    let first = store
        .workflow_run(&start.run_id)
        .expect("run reads")
        .expect("run exists");
    assert_eq!(first.revision, 1);
    assert_eq!(first.projection.status, RunStatus::Running);

    let cursor_event = WorkflowEvent::new(
        WorkflowEventId::new("event-2").expect("event id is valid"),
        start.run_id.clone(),
        cursor_event_payload(1),
    )
    .expect("cursor event is valid");
    let advanced = store
        .append_workflow_event(1, &cursor_event)
        .expect("cursor event appends");
    assert_eq!(advanced.revision, 2);
    assert_eq!(advanced.projection.cursor, 1);

    let stale = WorkflowEvent::new(
        WorkflowEventId::new("event-3").expect("event id is valid"),
        start.run_id.clone(),
        cursor_event_payload(2),
    )
    .expect("cursor event is valid");
    assert_eq!(
        store.append_workflow_event(1, &stale),
        Err(ExecutionStoreError::RevisionConflict)
    );
    let replayed = store
        .replay_workflow_run(&start.run_id)
        .expect("replay succeeds");
    assert_eq!(replayed, advanced);
    drop(store);

    let reopened = ExecutionStore::open(ExecutionStoreConfig::new(&path)).expect("reopens");
    assert_eq!(
        reopened
            .replay_workflow_run(&start.run_id)
            .expect("replay after reopen"),
        advanced
    );
    drop(reopened);
    remove_database(&path);
}

#[test]
fn invocation_intent_send_marker_and_completion_are_durable_and_atomic() {
    let path = database_path("invocation");
    let (mut store, start) = setup_store(&path);
    let invocation = WorkflowInvocation::new(
        WorkflowInvocationId::new("invocation-1").expect("invocation id is valid"),
        start.run_id.clone(),
        start.episode_id.clone(),
        start.plan_id.clone(),
        WorkflowCommandId::new("command-1").expect("command id is valid"),
        Some(GameOperationId::new("operation-1").expect("operation id is valid")),
        br#"{"command":"advance"}"#.to_vec(),
    )
    .expect("invocation is valid");
    let after_intent = store
        .record_workflow_invocation_intent(
            1,
            &invocation,
            WorkflowEventId::new("event-2").expect("event id is valid"),
        )
        .expect("intent commits");
    assert_eq!(after_intent.revision, 2);
    assert_eq!(
        store.record_workflow_invocation_intent(
            2,
            &invocation,
            WorkflowEventId::new("event-duplicate").expect("event id is valid"),
        ),
        Err(ExecutionStoreError::Conflict)
    );
    assert_eq!(
        store
            .workflow_run(&start.run_id)
            .expect("run reads")
            .expect("run exists")
            .revision,
        2
    );
    let reserved = store
        .workflow_invocation(&invocation.invocation_id)
        .expect("invocation reads")
        .expect("invocation exists");
    assert_eq!(reserved.state, InvocationState::Reserved);
    assert!(!reserved.send_marker);

    let sent = store
        .mark_workflow_invocation_sent(&invocation.invocation_id, reserved.revision)
        .expect("send marker commits");
    assert_eq!(sent.state, InvocationState::Sent);
    assert!(sent.send_marker);
    let (completed_run, completed_invocation) = store
        .complete_workflow_invocation(
            after_intent.revision,
            sent.revision,
            &invocation.invocation_id,
            InvocationOutcome::Accepted,
            WorkflowEventId::new("event-3").expect("event id is valid"),
        )
        .expect("completion commits");
    assert_eq!(completed_run.revision, 3);
    assert_eq!(completed_invocation.state, InvocationState::Accepted);
    assert!(completed_invocation.send_marker);
    assert_eq!(
        store
            .replay_workflow_run(&start.run_id)
            .expect("replay succeeds"),
        completed_run
    );
    drop(store);
    remove_database(&path);
}

#[test]
fn version_one_database_migrates_to_the_workflow_schema() {
    let path = database_path("migration");
    {
        let raw = Connection::open(&path).expect("migration fixture opens");
        raw.execute_batch(
            "CREATE TABLE store_metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
        )
        .expect("metadata table creates");
        raw.execute(
            "INSERT INTO store_metadata(key, value) VALUES ('recovery_contract', ?1)",
            [RECOVERY_CONTRACT_VERSION],
        )
        .expect("recovery contract writes");
        raw.execute_batch("PRAGMA user_version = 1")
            .expect("version one marker writes");
    }

    let migrated = ExecutionStore::open(ExecutionStoreConfig::new(&path)).expect("migrates");
    drop(migrated);
    let raw = Connection::open(&path).expect("migrated database opens");
    let version: i32 = raw
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version reads");
    let workflow_events: i64 = raw
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'workflow_events')",
            [],
            |row| row.get(0),
        )
        .expect("workflow table reads");
    assert_eq!(version, 7);
    assert_eq!(workflow_events, 1);
    drop(raw);
    remove_database(&path);
}

#[test]
fn blocked_external_work_does_not_hold_the_store_transaction() {
    let path = database_path("external-boundary");
    let (mut store, start) = setup_store(&path);
    let invocation = WorkflowInvocation::new(
        WorkflowInvocationId::new("invocation-1").expect("invocation id is valid"),
        start.run_id.clone(),
        start.episode_id.clone(),
        start.plan_id.clone(),
        WorkflowCommandId::new("command-1").expect("command id is valid"),
        None,
        b"external-request".to_vec(),
    )
    .expect("invocation is valid");
    let snapshot = store
        .record_workflow_invocation_intent(
            1,
            &invocation,
            WorkflowEventId::new("event-2").expect("event id is valid"),
        )
        .expect("intent commits before external work");
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let external = thread::spawn(move || {
        started_tx.send(()).expect("test channel is live");
        release_rx.recv().expect("external test is released");
    });
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("external blocks");

    let mut second_connection = ExecutionStore::open(ExecutionStoreConfig::new(&path))
        .expect("second process connection opens while external work blocks");
    let event = WorkflowEvent::new(
        WorkflowEventId::new("event-3").expect("event id is valid"),
        start.run_id.clone(),
        cursor_event_payload_with_invocation(1),
    )
    .expect("cursor event is valid");
    second_connection
        .append_workflow_event(snapshot.revision, &event)
        .expect("second writer is not blocked by external work");
    drop(second_connection);
    release_tx.send(()).expect("external test releases");
    external.join().expect("external test joins");
    drop(store);
    remove_database(&path);
}

#[test]
fn concurrent_writers_have_one_winner_for_a_run_revision() {
    let path = database_path("cas");
    let (store, start) = setup_store(&path);
    drop(store);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for suffix in ["a", "b"] {
        let barrier = Arc::clone(&barrier);
        let path = path.clone();
        let run_id = start.run_id.clone();
        handles.push(thread::spawn(move || {
            let mut store =
                ExecutionStore::open(ExecutionStoreConfig::new(path)).expect("writer opens");
            barrier.wait();
            let event = WorkflowEvent::new(
                WorkflowEventId::new(format!("event-2-{suffix}")).expect("event id is valid"),
                run_id,
                cursor_event_payload(1),
            )
            .expect("event is valid");
            store.append_workflow_event(1, &event)
        }));
    }
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("writer joins"))
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(ExecutionStoreError::RevisionConflict))
            .count(),
        1
    );
    remove_database(&path);
}

#[test]
fn settings_linked_sqlite_and_fail_closed_open_behavior_are_explicit() {
    let path = database_path("failure");
    let (mut store, _) = setup_store(&path);
    let pragmas = store.pragmas().expect("pragmas are readable");
    assert_eq!(pragmas.journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(pragmas.synchronous, 2);
    assert!(pragmas.foreign_keys);
    assert!(sqlite_version_at_least(3, 51, 1));
    store.close().expect("store closes");

    let raw = Connection::open(&path).expect("raw connection opens");
    raw.execute_batch("PRAGMA user_version = 999")
        .expect("newer schema marker writes");
    drop(raw);
    assert!(matches!(
        ExecutionStore::open(ExecutionStoreConfig::new(&path)),
        Err(ExecutionStoreError::Incompatible)
    ));
    remove_database(&path);

    let corrupt_path = database_path("corrupt");
    fs::write(&corrupt_path, b"not a sqlite database").expect("corrupt fixture writes");
    assert!(matches!(
        ExecutionStore::open(ExecutionStoreConfig::new(&corrupt_path)),
        Err(ExecutionStoreError::Corrupt)
    ));
    assert_eq!(
        fs::read(&corrupt_path).expect("corrupt fixture remains"),
        b"not a sqlite database"
    );
    remove_database(&corrupt_path);
}

fn sqlite_version_at_least(major: u32, minor: u32, patch: u32) -> bool {
    let mut components = ExecutionStore::sqlite_version().split('.');
    let current = (
        components
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0),
        components
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0),
        components
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0),
    );
    current >= (major, minor, patch)
}

fn cursor_event_payload(cursor: u64) -> WorkflowEventPayload {
    let mut counters = BTreeMap::new();
    counters.insert(String::from("run_started"), 1);
    WorkflowEventPayload::CursorAdvanced {
        cursor,
        stack: vec![String::from("step")],
        counters,
    }
}

fn cursor_event_payload_with_invocation(cursor: u64) -> WorkflowEventPayload {
    let mut counters = BTreeMap::new();
    counters.insert(String::from("run_started"), 1);
    counters.insert(String::from("invocations_reserved"), 1);
    WorkflowEventPayload::CursorAdvanced {
        cursor,
        stack: vec![String::from("step")],
        counters,
    }
}
