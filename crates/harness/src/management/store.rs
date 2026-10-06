// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::contract::{
    CommandRequest, CommandResponse, EventPage, ExportResponse, MANAGEMENT_SCHEMA_VERSION,
    MAX_STORE_BYTES, PendingOperation, PersistedStore, RunEvent, RunSnapshot,
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

pub trait WorkflowStore: Send + Sync {
    /// Whether this backend can atomically reserve seed candidates with their
    /// initial workflow run and durable submission identity.
    fn supports_durable_seed_bindings(&self) -> bool {
        false
    }

    /// Whether this backend can durably arbitrate an immutable key identity
    /// before a derive-once operation calculates its effective seed.
    fn supports_seed_operation_reservations(&self) -> bool {
        false
    }

    fn lookup_seed_operation(
        &self,
        _request_id: &str,
        _actor_digest: &str,
        _request_digest: &str,
    ) -> Result<SeedOperationLookup, StoreError> {
        Err(StoreError::new(
            "seed_operation_unavailable",
            "workflow store does not support durable seed operation reservations",
        ))
    }

    /// Insert or return the immutable winner. Implementations must compare
    /// actor/request/run/op/admission identity, but a losing provisional key
    /// identity is intentionally ignored in favor of the stored winner.
    fn reserve_seed_operation(
        &self,
        _proposed: SeedOperationRecord,
    ) -> Result<SeedOperationLookup, StoreError> {
        Err(StoreError::new(
            "seed_operation_unavailable",
            "workflow store does not support durable seed operation reservations",
        ))
    }

    fn lookup_seed_binding(
        &self,
        _request_id: &str,
        _actor_digest: &str,
        _request_digest: &str,
    ) -> Result<SeedBindingLookup, StoreError> {
        Err(StoreError::new(
            "seed_binding_unavailable",
            "workflow store does not support durable seed bindings",
        ))
    }

    fn create_seeded_run(
        &self,
        _request_id: &str,
        _request_digest: &str,
        _snapshot: RunSnapshot,
        _initial_events: Vec<RunEvent>,
        _operation: Option<SeedOperationRecord>,
        _seed_binding: SeedBindingRecord,
    ) -> Result<(), StoreError> {
        Err(StoreError::new(
            "seed_binding_unavailable",
            "workflow store does not support durable seed bindings",
        ))
    }

    fn read_seed_binding(
        &self,
        _workflow_run_id: &str,
    ) -> Result<Option<SeedBindingRecord>, StoreError> {
        Err(StoreError::new(
            "seed_binding_unavailable",
            "workflow store does not support durable seed bindings",
        ))
    }

    fn mark_seed_binding_awaiting_host_context(
        &self,
        _workflow_run_id: &str,
        _actor_digest: &str,
        _request_digest: &str,
    ) -> Result<SeedBindingRecord, StoreError> {
        Err(StoreError::new(
            "seed_binding_unavailable",
            "workflow store does not support durable seed bindings",
        ))
    }

    /// Whether this store atomically retains bounded context metadata with a
    /// command result. Unsupported stores cannot enable the opt-in service.
    fn supports_context_binding_history(&self) -> bool {
        false
    }

    /// Checks bounded history capacity before a new step crosses any owner or
    /// execution port. Exact command replay must bypass this admission check.
    fn check_context_binding_history_capacity(&self, _run_id: &str) -> Result<(), StoreError> {
        Err(StoreError::new(
            "context_history_unavailable",
            "context history is unsupported",
        ))
    }

    fn recorded_context_binding(
        &self,
        _run_id: &str,
        _node_execution_id: &str,
    ) -> Result<Option<super::RecordedContextBinding>, StoreError> {
        Err(StoreError::new(
            "context_history_unavailable",
            "context history is unsupported",
        ))
    }

    /// Commits historical evidence and result together, or neither. The binding
    /// must match the accepted pre-command cursor, not the advanced snapshot.
    fn apply_command_with_context_binding(
        &self,
        request: &CommandRequest,
        request_digest: &str,
        application: CommandApplication,
        record: super::RecordedContextBinding,
    ) -> Result<CommandResponse, StoreError> {
        let _ = (request, request_digest, application, record);
        Err(StoreError::new(
            "context_history_unavailable",
            "context history is unsupported",
        ))
    }

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

    /// Atomically replace the durable submission snapshot after the execution
    /// boundary settles. Implementations must preserve the request identity,
    /// definition digest, and exact target admission binding established by
    /// `create_run`.
    ///
    /// Stores that cannot update a reserved submission fail closed. Live
    /// execution callers must not treat an unsupported update as success.
    fn update_run_snapshot(
        &self,
        _request_id: &str,
        _request_digest: &str,
        _snapshot: RunSnapshot,
    ) -> Result<(), StoreError> {
        Err(StoreError::new(
            "snapshot_update_unavailable",
            "workflow store does not support reserved snapshot updates",
        ))
    }

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

    /// Persist an operation intent while its command remains in flight.
    ///
    /// The run revision is intentionally unchanged: command application still
    /// commits the next revision atomically after the effect settles or becomes
    /// explicitly unresolved.
    fn record_operation_intent(
        &self,
        _run_id: &str,
        _expected_revision: u64,
        _pending: PendingOperation,
    ) -> Result<(), StoreError> {
        Err(StoreError::new(
            "intent_persistence_unavailable",
            "workflow store does not support durable operation intents",
        ))
    }

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
