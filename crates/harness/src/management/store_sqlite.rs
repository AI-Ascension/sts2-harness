// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::Connection;

use super::super::contract::{
    CommandRequest, CommandResponse, EventPage, ExportResponse, PendingOperation, RunEvent,
    RunSnapshot,
};
use super::{CommandAcceptance, CommandApplication, StoreError, SubmissionLookup, WorkflowStore};
use super::{SeedBindingLookup, SeedBindingRecord, SeedOperationLookup, SeedOperationRecord};

#[path = "store_sqlite_context_history.rs"]
mod context_history;
#[path = "store_sqlite_ops.rs"]
mod ops;
#[path = "store_sqlite_runtime.rs"]
mod runtime;
#[path = "store_sqlite_seed_binding.rs"]
mod seed_binding;
#[path = "store_sqlite_seed_operation.rs"]
mod seed_operation;
#[path = "store_sqlite_seed_operation_lookup.rs"]
mod seed_operation_lookup;
#[path = "store_sqlite_support.rs"]
mod support;

const SQLITE_SCHEMA: &str = include_str!("store_sqlite_schema.sql");
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Transactional management state for the served loopback API.
///
/// The JSON file store remains available for fixture tests and migration compatibility. The
/// service entrypoint uses this store so run, event, and command acceptance survive process
/// restart under SQLite transactions.
pub struct SqliteWorkflowStore {
    pub(crate) connection: Mutex<Connection>,
}

impl SqliteWorkflowStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
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
        if path.is_file()
            && fs::metadata(path).map_err(support::io_error)?.len()
                > super::super::contract::MAX_STORE_BYTES as u64
        {
            return Err(StoreError::new(
                "store_too_large",
                "workflow store exceeds the supported bound",
            ));
        }
        let connection = Connection::open(path).map_err(support::sqlite_error)?;
        connection
            .busy_timeout(SQLITE_BUSY_TIMEOUT)
            .map_err(support::sqlite_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA foreign_keys = ON;",
            )
            .map_err(support::sqlite_error)?;
        connection
            .execute_batch(SQLITE_SCHEMA)
            .map_err(support::sqlite_error)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[cfg(test)]
    pub(crate) fn lookup_seed_operation_in_transaction_for_test(
        transaction: &rusqlite::Transaction<'_>,
        request_id: &str,
        actor_digest: &str,
        request_digest: &str,
    ) -> Result<SeedOperationLookup, StoreError> {
        seed_operation::lookup_in_transaction(transaction, request_id, actor_digest, request_digest)
    }
}

impl WorkflowStore for SqliteWorkflowStore {
    fn supports_durable_seed_bindings(&self) -> bool {
        true
    }

    fn supports_seed_operation_reservations(&self) -> bool {
        true
    }

    fn lookup_seed_operation(
        &self,
        request_id: &str,
        actor_digest: &str,
        request_digest: &str,
    ) -> Result<SeedOperationLookup, StoreError> {
        seed_operation::lookup(self, request_id, actor_digest, request_digest)
    }

    fn reserve_seed_operation(
        &self,
        proposed: SeedOperationRecord,
    ) -> Result<SeedOperationLookup, StoreError> {
        seed_operation::reserve(self, proposed)
    }

    fn lookup_seed_binding(
        &self,
        request_id: &str,
        actor_digest: &str,
        request_digest: &str,
    ) -> Result<SeedBindingLookup, StoreError> {
        seed_binding::lookup(self, request_id, actor_digest, request_digest)
    }

    fn create_seeded_run(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
        initial_events: Vec<RunEvent>,
        operation: Option<SeedOperationRecord>,
        seed_binding: SeedBindingRecord,
    ) -> Result<(), StoreError> {
        ops::create_seeded_run(
            self,
            request_id,
            request_digest,
            snapshot,
            initial_events,
            operation,
            seed_binding.into_record(),
        )
    }

    fn read_seed_binding(
        &self,
        workflow_run_id: &str,
    ) -> Result<Option<SeedBindingRecord>, StoreError> {
        seed_binding::read(self, workflow_run_id)
    }

    fn mark_seed_binding_awaiting_host_context(
        &self,
        workflow_run_id: &str,
        actor_digest: &str,
        request_digest: &str,
    ) -> Result<SeedBindingRecord, StoreError> {
        seed_binding::mark_awaiting_host_context(
            self,
            workflow_run_id,
            actor_digest,
            request_digest,
        )
    }

    fn supports_context_binding_history(&self) -> bool {
        true
    }

    fn check_context_binding_history_capacity(&self, run_id: &str) -> Result<(), StoreError> {
        context_history::check_capacity(self, run_id)
    }

    fn recorded_context_binding(
        &self,
        run_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<super::super::RecordedContextBinding>, StoreError> {
        context_history::read(self, run_id, node_execution_id)
    }

    fn apply_command_with_context_binding(
        &self,
        request: &CommandRequest,
        request_digest: &str,
        application: CommandApplication,
        record: super::super::RecordedContextBinding,
    ) -> Result<CommandResponse, StoreError> {
        ops::apply_command(self, request, request_digest, application, Some(record))
    }

    fn lookup_submission(
        &self,
        request_id: &str,
        request_digest: &str,
    ) -> Result<SubmissionLookup, StoreError> {
        ops::lookup_submission(self, request_id, request_digest)
    }

    fn create_run(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
        initial_events: Vec<RunEvent>,
    ) -> Result<(), StoreError> {
        ops::create_run(self, request_id, request_digest, snapshot, initial_events)
    }

    fn update_run_snapshot(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
    ) -> Result<(), StoreError> {
        ops::update_run_snapshot(self, request_id, request_digest, snapshot)
    }

    fn get_run(&self, run_id: &str) -> Result<Option<RunSnapshot>, StoreError> {
        ops::get_run(self, run_id)
    }

    fn events(
        &self,
        run_id: &str,
        after_sequence: u64,
        limit: u64,
    ) -> Result<EventPage, StoreError> {
        ops::events(self, run_id, after_sequence, limit)
    }

    fn accept_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<CommandAcceptance, StoreError> {
        ops::accept_command(self, request, request_digest)
    }

    fn apply_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
        application: CommandApplication,
    ) -> Result<CommandResponse, StoreError> {
        ops::apply_command(self, request, request_digest, application, None)
    }

    fn record_operation_intent(
        &self,
        run_id: &str,
        expected_revision: u64,
        pending: PendingOperation,
    ) -> Result<(), StoreError> {
        ops::record_operation_intent(self, run_id, expected_revision, pending)
    }

    fn release_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<(), StoreError> {
        ops::release_command(self, request, request_digest)
    }

    fn export(&self, run_id: &str, redacted: bool) -> Result<ExportResponse, StoreError> {
        ops::export(self, run_id, redacted)
    }
}
