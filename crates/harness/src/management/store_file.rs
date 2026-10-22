// SPDX-License-Identifier: MIT

use super::super::contract::{
    CommandRequest, CommandResponse, EventPage, ExportResponse, PendingOperation, RunEvent,
    RunSnapshot,
};
use super::ops::{
    accept_command, apply_command, create_run, events, export, get_run, lookup_submission,
    record_operation_intent, release_command, update_run_snapshot,
};
use super::{
    CommandAcceptance, CommandApplication, FileWorkflowStore, StoreError, SubmissionLookup,
    WorkflowStore,
};

impl WorkflowStore for FileWorkflowStore {
    fn claim_submission(
        &self,
        request_id: &str,
        claim: super::super::contract::SubmissionClaim,
    ) -> Result<super::super::contract::SubmissionClaimAcceptance, StoreError> {
        super::submission_claim::claim(&self.core, request_id, claim)
    }

    fn submission_claim(
        &self,
        request_id: &str,
    ) -> Result<Option<super::super::contract::SubmissionClaim>, StoreError> {
        super::submission_claim::read(&self.core, request_id)
    }

    fn finish_submission_claim(
        &self,
        request_id: &str,
        claim: super::super::contract::SubmissionClaim,
    ) -> Result<(), StoreError> {
        super::submission_claim::finish(&self.core, request_id, claim)
    }

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

    fn update_run_snapshot(
        &self,
        request_id: &str,
        request_digest: &str,
        snapshot: RunSnapshot,
    ) -> Result<(), StoreError> {
        update_run_snapshot(&self.core, request_id, request_digest, snapshot)
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

    fn record_operation_intent(
        &self,
        run_id: &str,
        expected_revision: u64,
        pending: PendingOperation,
    ) -> Result<(), StoreError> {
        record_operation_intent(&self.core, run_id, expected_revision, pending)
    }

    fn release_command(
        &self,
        request: &CommandRequest,
        request_digest: &str,
    ) -> Result<(), StoreError> {
        release_command(&self.core, request, request_digest)
    }

    fn export(&self, run_id: &str, redacted: bool) -> Result<ExportResponse, StoreError> {
        export(&self.core, run_id, redacted)
    }
}
