// SPDX-License-Identifier: MIT

use super::super::contract::{
    CommandRequest, CommandResponse, EventPage, ExportResponse, PendingOperation, RunEvent,
    RunSnapshot,
};
use super::{
    CommandAcceptance, CommandApplication, SeedBindingLookup, SeedBindingRecord,
    SeedOperationLookup, SeedOperationRecord, StoreError, SubmissionLookup,
};
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
    ) -> Result<Option<super::super::RecordedContextBinding>, StoreError> {
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
        record: super::super::RecordedContextBinding,
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
