// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::contract::{
    CommandResponse, MANAGEMENT_SCHEMA_VERSION, MAX_STORE_BYTES, PersistedStore, RunSnapshot,
};
use super::contract_seed_v2::{StoredSeedBindingV2, StoredSeedOperationV2};

#[path = "store_file.rs"]
mod file;
#[path = "store_memory.rs"]
mod memory;
#[path = "store_ops.rs"]
mod ops;
#[path = "store_sqlite.rs"]
mod sqlite;

#[path = "store_workflow.rs"]
mod workflow;
pub use workflow::WorkflowStore;

use ops::{io_store_error, persist};

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

pub enum SeedBindingLookup {
    Missing,
    Existing(Box<SeedBindingRecord>),
    Conflict,
}

pub enum SeedOperationLookup {
    Missing,
    Prepared(Box<SeedOperationRecord>),
    CandidatePersisted(Box<SeedBindingRecord>),
    Conflict,
}

/// Opaque persisted key-selection record. It contains only public key
/// identity/commitment metadata, never key material or the derived seed.
#[derive(Clone)]
pub struct SeedOperationRecord(StoredSeedOperationV2);

impl SeedOperationRecord {
    pub(crate) fn new(record: StoredSeedOperationV2) -> Self {
        Self(record)
    }

    pub(crate) fn record(&self) -> &StoredSeedOperationV2 {
        &self.0
    }

    pub(crate) fn into_record(self) -> StoredSeedOperationV2 {
        self.0
    }
}

/// Opaque store-owned seed material. Its public wrapper can cross the store
/// trait without exposing serialization, debug formatting, or record fields.
pub struct SeedBindingRecord(StoredSeedBindingV2);

impl SeedBindingRecord {
    pub(crate) fn new(record: StoredSeedBindingV2) -> Self {
        Self(record)
    }

    pub(crate) fn record(&self) -> &StoredSeedBindingV2 {
        &self.0
    }

    pub(crate) fn into_record(self) -> StoredSeedBindingV2 {
        self.0
    }
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
