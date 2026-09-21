// SPDX-License-Identifier: MIT

//! `#94`: the live teardown-failure reports on the command path.
//!
//! A live command whose teardown fails has to reach the operator as a distinct, machine-readable
//! outcome instead of being reported as the command it tried to perform. `cancel` normally settles
//! the run as `cancelled` and a terminal `step` normally settles it as `completed`; when the
//! session cannot be released the run is instead published as needing an operator with the
//! `live_cleanup_failed` reason code, so a supervisor that only inspects the response cannot
//! mistake a leaked episode for a clean stop.
//!
//! `live_workflow_recovery` already shows the *snapshot* side of one of these branches
//! (`cleanup_failure_is_visible_after_cancel` asserts the status and cleanup state), but the
//! published reason code — the stable discriminator a consumer switches on — was unmeasured: that
//! test would still pass if the branch published the plain `cancel` reason. These tests pin the
//! reason code on the command-applied event for both command sites, each against the control that
//! shows what the same command reports when the teardown works.
//!
//! The third site is on the submission path: a live submission whose durable admission snapshot
//! cannot be updated is refused after its session was already launched and registered, so the
//! teardown it owes the operator can fail too. That branch is pinned here for the same reason —
//! its error code is the only thing that distinguishes a leaked episode from a store outage —
//! with the control that shows the same store failure reported plainly when the teardown works.

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use sts2_harness::management::{
    AuthContext, CleanupState, CommandAcceptance, CommandKind, CommandOutcome, CommandRequest,
    CommandResponse, ErrorClass, EventPage, EventType, ExportResponse, LiveWorkflowOptions,
    LiveWorkflowSessionFactory, ManagementService, MemoryWorkflowStore, RunEvent, RunSnapshot,
    StoreCommandApplication, StoreError, SubmissionLookup, WorkflowRunStatus, WorkflowStore,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

/// The reason code the service published for one command, read off its `CommandApplied` event.
///
/// The command response carries no reason field, so the applied event is the durable record of
/// which outcome the command actually took.
fn applied_reason(
    service: &ManagementService,
    actor: &AuthContext,
    run_id: &str,
    sequence: u64,
) -> (EventType, String) {
    let event = service
        .events(actor, run_id, 0, 128)
        .expect("events")
        .events
        .into_iter()
        .find(|event| event.sequence == sequence)
        .expect("applied event");
    (event.event_type, event.payload.reason_code)
}

fn live_service_with(factory: Arc<FakeFactory>) -> ManagementService {
    live_service_with_store(Arc::new(MemoryWorkflowStore::new()), factory)
}

fn live_service_with_store(
    store: Arc<dyn WorkflowStore>,
    factory: Arc<FakeFactory>,
) -> ManagementService {
    live_service(
        store,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service")
}

/// The reservation store that refuses the durable admission snapshot.
///
/// A live submission is admitted, opened and launched before the service writes the settled
/// admission back to the submission record, so a store that cannot make that write refuses the
/// caller *after* the session exists. Only the first update fails: the reservation's own recovery
/// mark has to succeed, or the failure reported would be the persistence one rather than the
/// teardown one this site exists to surface.
struct RefusingSnapshotStore {
    updates: Mutex<usize>,
}

impl RefusingSnapshotStore {
    fn new() -> Self {
        Self {
            updates: Mutex::new(0),
        }
    }
}

fn unused(surface: &str) -> StoreError {
    StoreError {
        code: "test_store_unused".to_owned(),
        message: format!("{surface} are not part of the submission cleanup fence"),
    }
}

impl WorkflowStore for RefusingSnapshotStore {
    fn lookup_submission(
        &self,
        _request_id: &str,
        _request_digest: &str,
    ) -> Result<SubmissionLookup, StoreError> {
        Ok(SubmissionLookup::Missing)
    }

    fn create_run(
        &self,
        _request_id: &str,
        _request_digest: &str,
        _snapshot: RunSnapshot,
        _initial_events: Vec<RunEvent>,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    fn update_run_snapshot(
        &self,
        _request_id: &str,
        _request_digest: &str,
        _snapshot: RunSnapshot,
    ) -> Result<(), StoreError> {
        let mut updates = self.updates.lock().expect("update lock");
        *updates += 1;
        if *updates == 1 {
            return Err(StoreError {
                code: "snapshot_update_unavailable".to_owned(),
                message: "workflow store does not support reserved snapshot updates".to_owned(),
            });
        }
        Ok(())
    }

    fn get_run(&self, _run_id: &str) -> Result<Option<RunSnapshot>, StoreError> {
        Ok(None)
    }

    fn events(
        &self,
        _run_id: &str,
        _after_sequence: u64,
        _limit: u64,
    ) -> Result<EventPage, StoreError> {
        Err(unused("events"))
    }

    fn accept_command(
        &self,
        _request: &CommandRequest,
        _request_digest: &str,
    ) -> Result<CommandAcceptance, StoreError> {
        Err(unused("commands"))
    }

    fn apply_command(
        &self,
        _request: &CommandRequest,
        _request_digest: &str,
        _application: StoreCommandApplication,
    ) -> Result<CommandResponse, StoreError> {
        Err(unused("commands"))
    }

    fn release_command(
        &self,
        _request: &CommandRequest,
        _request_digest: &str,
    ) -> Result<(), StoreError> {
        Err(unused("commands"))
    }

    fn export(&self, _run_id: &str, _redacted: bool) -> Result<ExportResponse, StoreError> {
        Err(unused("export"))
    }
}

#[test]
fn cancel_reports_a_failed_teardown_instead_of_a_clean_cancellation() {
    // The lease release fails, so the run cannot be left running and cannot be left cleanly
    // cancelled either: the operator has to be told which of the two happened.
    let factory = Arc::new(FakeFactory::cleanup_error());
    let service = live_service_with(Arc::clone(&factory));
    let actor = actor();
    let run_id = service
        .submit_run(
            &actor,
            request("request-live-cancel-cleanup-fault", definition(false)),
        )
        .expect("submit")
        .workflow_run_id;

    let cancelled = service
        .command(&actor, command(&run_id, "cancel", 1, CommandKind::Cancel))
        .expect("cancel response");
    assert_eq!(cancelled.outcome, CommandOutcome::Applied);
    assert_eq!(cancelled.run_revision, 2);

    let (kind, reason) = applied_reason(
        &service,
        &actor,
        &run_id,
        cancelled.sequence.expect("sequence"),
    );
    assert_eq!(kind, EventType::CommandApplied);
    assert_eq!(
        reason, "live_cleanup_failed",
        "a cancellation whose teardown failed is not published as a clean cancellation"
    );

    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::NeedsOperator);
    assert_eq!(snapshot.cleanup, CleanupState::NeedsOperator);
    assert_eq!(
        factory.entries(),
        ["launch", "stop", "release"],
        "the refusal attempted the teardown that failed"
    );
}

#[test]
fn a_cancel_whose_teardown_succeeds_reports_the_plain_cancellation() {
    // The control for the test above: the identical command with a working teardown must report
    // `cancel`. If the cleanup reporting were made unconditional this test would fail, and if it
    // were removed the test above would see this reason.
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_service_with(Arc::clone(&factory));
    let actor = actor();
    let run_id = service
        .submit_run(
            &actor,
            request("request-live-cancel-cleanup-control", definition(false)),
        )
        .expect("submit")
        .workflow_run_id;

    let cancelled = service
        .command(&actor, command(&run_id, "cancel", 1, CommandKind::Cancel))
        .expect("cancel response");
    let (_, reason) = applied_reason(
        &service,
        &actor,
        &run_id,
        cancelled.sequence.expect("sequence"),
    );
    assert_eq!(reason, "cancel");

    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::Cancelled);
    assert_eq!(snapshot.cleanup, CleanupState::Complete);
}

#[test]
fn terminal_step_reports_a_failed_teardown_instead_of_completion() {
    // The authored graph reaches its terminal node on the fourth step, so that step is the one
    // that releases the lease. A release that fails there must not be published as `step`, which
    // is what the run would report if the terminal release succeeded.
    let factory = Arc::new(FakeFactory::cleanup_error());
    let service = live_service_with(Arc::clone(&factory));
    let actor = actor();
    let run_id = service
        .submit_run(
            &actor,
            request("request-live-terminal-cleanup-fault", definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    for (id, revision) in [("step-1", 1), ("step-2", 2), ("step-3", 3)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }

    let terminal = service
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("terminal step");
    assert_eq!(terminal.outcome, CommandOutcome::Applied);
    assert_eq!(terminal.run_revision, 5);

    let (kind, reason) = applied_reason(
        &service,
        &actor,
        &run_id,
        terminal.sequence.expect("sequence"),
    );
    assert_eq!(kind, EventType::CommandApplied);
    assert_eq!(
        reason, "live_cleanup_failed",
        "a terminal step whose teardown failed is not published as a completed step"
    );

    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::NeedsOperator);
    assert_eq!(snapshot.cleanup, CleanupState::NeedsOperator);
    assert_eq!(
        factory.entries().last().map(String::as_str),
        Some("release"),
        "the terminal step reached the release it could not complete"
    );
}

#[test]
fn a_terminal_step_whose_teardown_succeeds_reports_completion() {
    // The control for the test above: the same four steps with a working release must complete the
    // run and publish the plain `step` reason.
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_service_with(Arc::clone(&factory));
    let actor = actor();
    let run_id = service
        .submit_run(
            &actor,
            request("request-live-terminal-cleanup-control", definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    for (id, revision) in [("step-1", 1), ("step-2", 2), ("step-3", 3)] {
        service
            .command(&actor, command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }

    let terminal = service
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("terminal step");
    let (_, reason) = applied_reason(
        &service,
        &actor,
        &run_id,
        terminal.sequence.expect("sequence"),
    );
    assert_eq!(reason, "step");

    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.status, WorkflowRunStatus::Completed);
    assert_eq!(snapshot.cleanup, CleanupState::Complete);
}

#[test]
fn a_submission_refused_after_launch_reports_a_failed_teardown_instead_of_a_store_outage() {
    // The store cannot record the settled admission, so the submission is refused once the
    // session is already open and registered. Its teardown fails as well, which has to reach the
    // operator as a distinct outcome rather than as the plain store failure: the run is still
    // there and a caller told only "the store refused" would leave it behind.
    let factory = Arc::new(FakeFactory::cleanup_error());
    let service =
        live_service_with_store(Arc::new(RefusingSnapshotStore::new()), Arc::clone(&factory));
    let actor = actor();

    let error = service
        .submit_run(
            &actor,
            request("request-live-submission-cleanup-fault", definition(false)),
        )
        .expect_err("a submission whose settled admission cannot be recorded must be refused");
    assert_eq!(error.code, "live_submission_cleanup_failed");
    assert_eq!(error.class, ErrorClass::Unavailable);
    assert!(
        error.message.contains("snapshot_update_unavailable"),
        "the store failure the caller has to reason about is preserved: {}",
        error.message
    );
    assert!(
        error.message.contains("fake_release"),
        "the teardown failure that could not be completed is preserved: {}",
        error.message
    );
    assert_eq!(
        factory.entries(),
        ["launch", "stop", "release"],
        "the refusal attempted the teardown of the session it had already launched"
    );
}

#[test]
fn a_submission_refused_after_launch_whose_teardown_succeeds_reports_the_store_failure() {
    // The control for the test above: the identical store failure with a working teardown must be
    // reported as the store failure itself. If the cleanup reporting were made unconditional this
    // test would fail, and if it were removed the test above would see this code.
    let factory = Arc::new(FakeFactory::new(false));
    let service =
        live_service_with_store(Arc::new(RefusingSnapshotStore::new()), Arc::clone(&factory));
    let actor = actor();

    let error = service
        .submit_run(
            &actor,
            request("request-live-submission-cleanup-control", definition(false)),
        )
        .expect_err("a submission whose settled admission cannot be recorded must be refused");
    assert_eq!(error.code, "snapshot_update_unavailable");
    assert_eq!(error.class, ErrorClass::Store);
    assert!(
        !error.message.contains("cleanup failed"),
        "a teardown that succeeded is not reported as a failure: {}",
        error.message
    );
    assert_eq!(
        factory.entries(),
        ["launch", "stop", "release"],
        "the refusal still attempted the teardown"
    );
}
