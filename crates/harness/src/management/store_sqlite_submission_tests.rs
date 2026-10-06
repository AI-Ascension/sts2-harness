// SPDX-License-Identifier: MIT

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::management::SqliteWorkflowStore;
use crate::management::contract::{
    Budget, CleanupState, Cursor, GameOutcome, RUN_SCHEMA_VERSION, RunSnapshot, WorkflowRunStatus,
};

const RUN_ID: &str = "run-submission-race";
const REQUEST_ID: &str = "request-submission-race";
const REQUEST_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEST_WAIT: Duration = Duration::from_secs(3);

static NEXT_DATABASE_ID: AtomicU64 = AtomicU64::new(0);
static BUSY_HANDLER_TEST_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UpdateStage {
    WaitingToAcquireWriter,
    TransactionAcquired,
}

struct BusyHandlerState {
    stage_sender: mpsc::SyncSender<UpdateStage>,
    release_receiver: mpsc::Receiver<()>,
    notified: bool,
}

static BUSY_HANDLER_STATE: Mutex<Option<BusyHandlerState>> = Mutex::new(None);

fn signal_busy_handler(_attempt: i32) -> bool {
    let Ok(mut active_state) = BUSY_HANDLER_STATE.lock() else {
        return false;
    };
    let Some(state) = active_state.as_mut() else {
        return false;
    };
    if !state.notified {
        state.notified = true;
        if state
            .stage_sender
            .try_send(UpdateStage::WaitingToAcquireWriter)
            .is_err()
        {
            return false;
        }
        return state.release_receiver.recv_timeout(TEST_WAIT).is_ok();
    }
    false
}

struct TemporaryDatabase {
    path: PathBuf,
}

impl TemporaryDatabase {
    fn create() -> Result<Self, String> {
        for _ in 0..128 {
            let id = NEXT_DATABASE_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "h103-submission-race-{}-{id}.sqlite",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    drop(file);
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(format!("create temporary SQLite fixture: {error}")),
            }
        }
        Err("could not allocate a unique temporary SQLite fixture".to_owned())
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        let mut sidecars = vec![self.path.clone()];
        for suffix in ["-wal", "-shm"] {
            let mut name = self.path.as_os_str().to_owned();
            name.push(suffix);
            sidecars.push(PathBuf::from(name));
        }
        for path in sidecars {
            let _ = fs::remove_file(path);
        }
    }
}

fn snapshot() -> RunSnapshot {
    RunSnapshot {
        schema_version: RUN_SCHEMA_VERSION.to_owned(),
        workflow_run_id: RUN_ID.to_owned(),
        definition_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_owned(),
        run_revision: 1,
        status: WorkflowRunStatus::Running,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: "graph".to_owned(),
            node_id: "node".to_owned(),
            node_execution_id: "node-execution".to_owned(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
        admission: None,
        execution_mode: None,
    }
}

fn create_store(
    database: &TemporaryDatabase,
) -> Result<(SqliteWorkflowStore, RunSnapshot), String> {
    let store = SqliteWorkflowStore::open(database.path()).map_err(|error| error.to_string())?;
    let initial = snapshot();
    let bytes = serde_json::to_vec(&initial).map_err(|error| error.to_string())?;
    let connection = store
        .connection
        .lock()
        .map_err(|error| format!("lock SQLite test store: {error}"))?;
    connection
        .execute(
            "INSERT INTO management_submissions(request_id, request_digest, workflow_run_id)
             VALUES (?1, ?2, ?3)",
            params![REQUEST_ID, REQUEST_DIGEST, RUN_ID],
        )
        .map_err(|error| format!("insert submission index: {error}"))?;
    connection
        .execute(
            "INSERT INTO management_runs(
                workflow_run_id, request_id, request_digest, snapshot, oldest_sequence
             ) VALUES (?1, ?2, ?3, ?4, 1)",
            params![RUN_ID, REQUEST_ID, REQUEST_DIGEST, bytes],
        )
        .map_err(|error| format!("insert workflow snapshot: {error}"))?;
    drop(connection);
    Ok((store, initial))
}

struct UpdateWorker {
    join_handle: JoinHandle<Result<(), crate::management::StoreError>>,
    ready_receiver: mpsc::Receiver<()>,
    stage_receiver: mpsc::Receiver<UpdateStage>,
    busy_release_sender: mpsc::SyncSender<()>,
    resume_sender: mpsc::SyncSender<()>,
}

struct UpdateObservation {
    ready: bool,
    first_stage: Option<UpdateStage>,
    acquired_after_commit: Option<UpdateStage>,
}

fn install_busy_handler_state(
    stage_sender: mpsc::SyncSender<UpdateStage>,
    release_receiver: mpsc::Receiver<()>,
) -> Result<(), String> {
    let mut busy_handler_state = BUSY_HANDLER_STATE
        .lock()
        .map_err(|error| format!("lock busy-handler fixture state: {error}"))?;
    if busy_handler_state.is_some() {
        return Err("busy-handler fixture state must be empty before this test".to_owned());
    }
    *busy_handler_state = Some(BusyHandlerState {
        stage_sender,
        release_receiver,
        notified: false,
    });
    Ok(())
}

fn launch_update_worker(
    store: Arc<SqliteWorkflowStore>,
    updated: RunSnapshot,
) -> Result<UpdateWorker, String> {
    let (stage_sender, stage_receiver) = mpsc::sync_channel(2);
    let (resume_sender, resume_receiver) = mpsc::sync_channel(1);
    let (busy_release_sender, busy_release_receiver) = mpsc::sync_channel(1);
    install_busy_handler_state(stage_sender.clone(), busy_release_receiver)?;

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let worker_store = Arc::clone(&store);
    let worker_stage_sender = stage_sender.clone();
    let worker = thread::Builder::new()
        .name("h103-snapshot-update-test".to_owned())
        .spawn(move || {
            let _ = ready_sender.send(());
            super::update_run_snapshot_with_after_begin(
                &worker_store,
                REQUEST_ID,
                REQUEST_DIGEST,
                updated,
                move || {
                    let _ = worker_stage_sender.send(UpdateStage::TransactionAcquired);
                    let _ = resume_receiver.recv_timeout(TEST_WAIT);
                },
            )
        });
    let join_handle = match worker {
        Ok(worker) => worker,
        Err(error) => {
            let cleanup_result = BUSY_HANDLER_STATE
                .lock()
                .map(|mut state| *state = None)
                .map_err(|lock_error| {
                    format!("clear busy-handler state after spawn failure: {lock_error}")
                });
            cleanup_result?;
            return Err(format!("spawn snapshot updater: {error}"));
        }
    };

    Ok(UpdateWorker {
        join_handle,
        ready_receiver,
        stage_receiver,
        busy_release_sender,
        resume_sender,
    })
}

fn finish_update_worker(
    worker: UpdateWorker,
    competing_transaction: rusqlite::Transaction<'_>,
) -> Result<UpdateObservation, String> {
    let ready = worker.ready_receiver.recv_timeout(TEST_WAIT).is_ok();
    let first_stage = worker.stage_receiver.recv_timeout(TEST_WAIT).ok();
    let writer_commit = competing_transaction
        .commit()
        .map_err(|error| format!("commit competing WAL writer: {error}"));
    let _ = worker.busy_release_sender.send(());
    let acquired_after_commit = if first_stage == Some(UpdateStage::TransactionAcquired) {
        None
    } else {
        worker.stage_receiver.recv_timeout(TEST_WAIT).ok()
    };
    let _ = worker.resume_sender.send(());
    let worker_result = worker.join_handle.join();
    let cleanup_result = BUSY_HANDLER_STATE
        .lock()
        .map(|mut state| *state = None)
        .map_err(|error| format!("lock busy-handler fixture state for cleanup: {error}"));

    cleanup_result?;
    let update_result = worker_result
        .map_err(|_| "snapshot updater worker panicked".to_owned())?
        .map_err(|error| format!("snapshot update after writer commit: {error:?}"));
    writer_commit?;
    update_result?;

    Ok(UpdateObservation {
        ready,
        first_stage,
        acquired_after_commit,
    })
}

fn verify_updated_snapshot(store: &SqliteWorkflowStore) -> Result<(), String> {
    let connection = store
        .connection
        .lock()
        .map_err(|error| format!("lock SQLite test store for readback: {error}"))?;
    let bytes: Vec<u8> = connection
        .query_row(
            "SELECT snapshot FROM management_runs WHERE workflow_run_id = ?1",
            [RUN_ID],
            |row| row.get(0),
        )
        .map_err(|error| format!("read updated workflow snapshot: {error}"))?;
    let stored: RunSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode updated snapshot: {error}"))?;
    assert_eq!(stored.status, WorkflowRunStatus::Paused);
    let oldest_sequence: i64 = connection
        .query_row(
            "SELECT oldest_sequence FROM management_runs WHERE workflow_run_id = ?1",
            [RUN_ID],
            |row| row.get(0),
        )
        .map_err(|error| format!("read competing writer marker: {error}"))?;
    assert_eq!(oldest_sequence, 2);
    Ok(())
}

#[test]
fn update_snapshot_waits_for_writer_before_reading_and_then_succeeds() -> Result<(), String> {
    let _test_guard = BUSY_HANDLER_TEST_LOCK
        .lock()
        .map_err(|error| format!("serialize busy-handler fixture: {error}"))?;
    let database = TemporaryDatabase::create()?;
    let (store, mut updated) = create_store(&database)?;
    updated.status = WorkflowRunStatus::Paused;
    store
        .connection
        .lock()
        .map_err(|error| format!("lock SQLite test store: {error}"))?
        .busy_handler(Some(signal_busy_handler))
        .map_err(|error| format!("install per-connection busy handler: {error}"))?;

    let mut competing_connection = Connection::open(database.path())
        .map_err(|error| format!("open competing SQLite connection: {error}"))?;
    let competing_transaction = competing_connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("acquire competing writer: {error}"))?;
    competing_transaction
        .execute(
            "UPDATE management_runs SET oldest_sequence = 2 WHERE workflow_run_id = ?1",
            [RUN_ID],
        )
        .map_err(|error| format!("hold competing WAL write: {error}"))?;

    let worker_store = Arc::new(store);
    let worker = launch_update_worker(Arc::clone(&worker_store), updated)?;
    let observation = finish_update_worker(worker, competing_transaction)?;
    assert!(
        observation.ready,
        "updater worker did not start within the bound"
    );
    assert_eq!(
        observation.first_stage,
        Some(UpdateStage::WaitingToAcquireWriter),
        "IMMEDIATE update must wait before reading while another writer owns the database"
    );
    assert_eq!(
        observation.acquired_after_commit,
        Some(UpdateStage::TransactionAcquired),
        "snapshot transaction should begin only after the competing commit"
    );
    verify_updated_snapshot(&worker_store)
}

#[test]
fn deferred_reader_upgrade_reports_busy_snapshot_after_rival_commit() -> Result<(), String> {
    let database = TemporaryDatabase::create()?;
    let (store, _) = create_store(&database)?;
    drop(store);

    let mut reader = Connection::open(database.path())
        .map_err(|error| format!("open deferred reader: {error}"))?;
    let reader_transaction = reader
        .transaction()
        .map_err(|error| format!("begin deferred read transaction: {error}"))?;
    let before: i64 = reader_transaction
        .query_row(
            "SELECT oldest_sequence FROM management_runs WHERE workflow_run_id = ?1",
            [RUN_ID],
            |row| row.get(0),
        )
        .map_err(|error| format!("establish reader snapshot: {error}"))?;
    assert_eq!(before, 1);

    let mut writer = Connection::open(database.path())
        .map_err(|error| format!("open competing writer: {error}"))?;
    let writer_transaction = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("begin immediate writer: {error}"))?;
    writer_transaction
        .execute(
            "UPDATE management_runs SET oldest_sequence = 2 WHERE workflow_run_id = ?1",
            [RUN_ID],
        )
        .map_err(|error| format!("write newer WAL snapshot: {error}"))?;
    writer_transaction
        .commit()
        .map_err(|error| format!("commit competing writer: {error}"))?;

    let extended_code = match reader_transaction.execute(
        "UPDATE management_runs SET oldest_sequence = 3 WHERE workflow_run_id = ?1",
        [RUN_ID],
    ) {
        Ok(_) => return Err("deferred stale reader unexpectedly promoted to a writer".to_owned()),
        Err(rusqlite::Error::SqliteFailure(code, _)) => code.extended_code,
        Err(other) => {
            return Err(format!(
                "expected SQLite busy-snapshot failure, got {other:?}"
            ));
        }
    };
    assert_eq!(extended_code, rusqlite::ffi::SQLITE_BUSY_SNAPSHOT);
    Ok(())
}
