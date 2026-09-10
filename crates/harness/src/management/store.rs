// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::contract::{
    CommandRequest, CommandResponse, EventPage, ExportResponse, MANAGEMENT_SCHEMA_VERSION,
    MAX_STORE_BYTES, PersistedStore, RunEvent, RunSnapshot,
};

#[path = "store_file.rs"]
mod file;
#[path = "store_ops.rs"]
mod ops;
#[path = "store_sqlite.rs"]
mod sqlite;

use ops::{
    accept_command, apply_command, create_run, events, export, get_run, io_store_error,
    lookup_submission, persist, release_command,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError {
    pub code: String,
    pub message: String,
}

impl StoreError {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionLookup {
    Missing,
    Existing(Box<RunSnapshot>),
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandAcceptance {
    New {
        snapshot: RunSnapshot,
        requested_sequence: u64,
    },
    Existing {
        snapshot: RunSnapshot,
        response: Option<CommandResponse>,
        application_in_flight: bool,
    },
    Conflict,
}

#[derive(Clone, Debug)]
pub struct CommandApplication {
    pub snapshot: RunSnapshot,
    pub outcome: super::contract::CommandOutcome,
    pub reason_code: String,
}

pub trait WorkflowStore: Send + Sync {
    fn lookup_submission(
        &self,
        request_id: &str,
        request_digest: &str,
    ) -> Result<SubmissionLookup, StoreError>;

    fn create_run(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
        initial_events: Vec<RunEvent>,
    ) -> Result<(), StoreError>;

    fn get_run(&self, run_id: &str) -> Result<Option<RunSnapshot>, StoreError>;

    fn events(
        &self,
        run_id: &str,
        after_sequence: u64,
        limit: u64,
    ) -> Result<EventPage, StoreError>;

    fn accept_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<CommandAcceptance, StoreError>;

    fn apply_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
        application: CommandApplication,
    ) -> Result<CommandResponse, StoreError>;

    fn release_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<(), StoreError>;

    fn export(&self, run_id: &str, redacted: bool) -> Result<ExportResponse, StoreError>;
}

struct StoreCore {
    state: Mutex<PersistedStore>,
    path: Option<PathBuf>,
}

impl StoreCore {
    fn memory() -> Self {
        Self {
            state: Mutex::new(PersistedStore::empty()),
            path: None,
        }
    }

    fn file(path: PathBuf, state: PersistedStore) -> Self {
        Self {
            state: Mutex::new(state),
            path: Some(path),
        }
    }

    fn read(&self) -> Result<std::sync::MutexGuard<'_, PersistedStore>, StoreError> {
        self.state
            .lock()
            .map_err(|_| StoreError::new("store_poisoned", "workflow store lock is poisoned"))
    }

    fn mutate<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut PersistedStore) -> Result<T, StoreError>,
    {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::new("store_poisoned", "workflow store lock is poisoned"))?;
        let before = state.clone();
        let result = operation(&mut state);
        if result.is_ok()
            && let Some(path) = &self.path
            && let Err(error) = persist(path, &state)
        {
            *state = before;
            return Err(error);
        }
        result
    }
}

pub struct MemoryWorkflowStore {
    core: StoreCore,
}

impl MemoryWorkflowStore {
    pub fn new() -> Self {
        Self {
            core: StoreCore::memory(),
        }
    }
}

impl Default for MemoryWorkflowStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FileWorkflowStore {
    core: StoreCore,
}

pub use sqlite::SqliteWorkflowStore;

impl FileWorkflowStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if path.as_os_str().is_empty() {
            return Err(StoreError::new(
                "invalid_store_path",
                "workflow store path is empty",
            ));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(StoreError::new(
                "store_parent_missing",
                "workflow store parent directory does not exist",
            ));
        }
        let state = if path.exists() {
            let metadata = fs::metadata(&path).map_err(io_store_error)?;
            if !metadata.is_file() {
                return Err(StoreError::new(
                    "invalid_store_path",
                    "workflow store path is not a regular file",
                ));
            }
            if metadata.len() > MAX_STORE_BYTES as u64 {
                return Err(StoreError::new(
                    "store_too_large",
                    "workflow store exceeds the supported bound",
                ));
            }
            let bytes = fs::read(&path).map_err(io_store_error)?;
            serde_json::from_slice::<PersistedStore>(&bytes).map_err(|error| {
                StoreError::new(
                    "store_corrupt",
                    format!("workflow store is invalid: {error}"),
                )
            })?
        } else {
            let state = PersistedStore::empty();
            persist(&path, &state)?;
            state
        };
        if state.schema_version != MANAGEMENT_SCHEMA_VERSION {
            return Err(StoreError::new(
                "store_schema_unsupported",
                "workflow store schema is unsupported",
            ));
        }
        Ok(Self {
            core: StoreCore::file(path, state),
        })
    }
}

impl WorkflowStore for MemoryWorkflowStore {
    fn lookup_submission(
        &self,
        request_id: &str,
        request_digest: &str,
    ) -> Result<SubmissionLookup, StoreError> {
        ops::lookup_submission(&self.core, request_id, request_digest)
    }

    fn create_run(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
        initial_events: Vec<RunEvent>,
    ) -> Result<(), StoreError> {
        ops::create_run(
            &self.core,
            request_id,
            request_digest,
            snapshot,
            initial_events,
        )
    }

    fn get_run(&self, run_id: &str) -> Result<Option<RunSnapshot>, StoreError> {
        ops::get_run(&self.core, run_id)
    }

    fn events(
        &self,
        run_id: &str,
        after_sequence: u64,
        limit: u64,
    ) -> Result<EventPage, StoreError> {
        ops::events(&self.core, run_id, after_sequence, limit)
    }

    fn accept_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<CommandAcceptance, StoreError> {
        ops::accept_command(&self.core, request, request_digest)
    }

    fn apply_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
        application: CommandApplication,
    ) -> Result<CommandResponse, StoreError> {
        ops::apply_command(&self.core, request, request_digest, application)
    }

    fn release_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<(), StoreError> {
        ops::release_command(&self.core, request, request_digest)
    }

    fn export(&self, run_id: &str, redacted: bool) -> Result<ExportResponse, StoreError> {
        ops::export(&self.core, run_id, redacted)
    }
}
