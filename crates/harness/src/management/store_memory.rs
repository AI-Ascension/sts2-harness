// SPDX-License-Identifier: MIT

use super::super::contract::{
    CommandRequest, CommandResponse, EventPage, ExportResponse, PendingOperation,
};
use super::ops;
use super::{
    CommandAcceptance, CommandApplication, MemoryWorkflowStore, RunEvent, RunSnapshot, StoreError,
    SubmissionLookup, WorkflowStore,
};

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

    fn update_run_snapshot(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
    ) -> Result<(), StoreError> {
        ops::update_run_snapshot(&self.core, request_id, request_digest, snapshot)
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

    fn record_operation_intent(
        &self,
        run_id: &str,
        expected_revision: u64,
        pending: PendingOperation,
    ) -> Result<(), StoreError> {
        ops::record_operation_intent(&self.core, run_id, expected_revision, pending)
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
