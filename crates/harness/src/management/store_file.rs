// SPDX-License-Identifier: MIT

use super::{
    CommandAcceptance, CommandApplication, CommandRequest, CommandResponse, EventPage,
    ExportResponse, FileWorkflowStore, RunEvent, RunSnapshot, StoreError, SubmissionLookup,
    WorkflowStore, accept_command, apply_command, create_run, events, export, get_run,
    lookup_submission,
};

impl WorkflowStore for FileWorkflowStore {
    fn lookup_submission(
        &self,
        request_id: &str,
        request_digest: &str,
    ) -> Result<SubmissionLookup, StoreError> {
        lookup_submission(&self.core, request_id, request_digest)
    }

    fn create_run(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
        initial_events: Vec<RunEvent>,
    ) -> Result<(), StoreError> {
        create_run(
            &self.core,
            request_id,
            request_digest,
            snapshot,
            initial_events,
        )
    }

    fn get_run(&self, run_id: &str) -> Result<Option<RunSnapshot>, StoreError> {
        get_run(&self.core, run_id)
    }

    fn events(
        &self,
        run_id: &str,
        after_sequence: u64,
        limit: u64,
    ) -> Result<EventPage, StoreError> {
        events(&self.core, run_id, after_sequence, limit)
    }

    fn accept_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<CommandAcceptance, StoreError> {
        accept_command(&self.core, request, request_digest)
    }

    fn apply_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
        application: CommandApplication,
    ) -> Result<CommandResponse, StoreError> {
        apply_command(&self.core, request, request_digest, application)
    }

    fn export(&self, run_id: &str, redacted: bool) -> Result<ExportResponse, StoreError> {
        export(&self.core, run_id, redacted)
    }
}
