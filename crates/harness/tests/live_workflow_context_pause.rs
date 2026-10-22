// SPDX-License-Identifier: MIT

//! Coordinator component evidence for a clean owner-pause refusal. The session wrapper reports
//! typed boundary outcomes; production owner admission is tested in the runtime binary and the
//! separate Console process witness. No fixture here claims native owner or provider authority.

#![allow(clippy::expect_used)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_PAUSED, CleanupState, CommandKind, CommandOutcome,
    ContextOwnerControlLimits, ErrorClass, EventClassification, LiveWorkflowOptions,
    LiveWorkflowSession, LiveWorkflowSessionFactory, ManagementError, MemoryWorkflowStore,
    RunRequest, SqliteWorkflowStore, TargetCatalogResponse, WorkflowRunStatus, WorkflowStore,
};
use sts2_harness::{
    ActionIdentity, Decision, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, TransitionReceipt, WaitSample,
};

#[path = "support/live_workflow.rs"]
mod support;
use support::*;

struct RefusingFactory {
    inner: Arc<FakeFactory>,
    refusal: Arc<Mutex<Option<ManagementError>>>,
    attempts: Arc<AtomicUsize>,
}

impl RefusingFactory {
    fn new(class: ErrorClass) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(FakeFactory::new(false)),
            refusal: Arc::new(Mutex::new(Some(ManagementError::new(
                class,
                CONTEXT_OWNER_PAUSED,
                "synthetic decision boundary outcome",
            )))),
            attempts: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn wrap(&self, inner: Box<dyn LiveWorkflowSession>) -> Box<dyn LiveWorkflowSession> {
        Box::new(RefusingSession {
            inner,
            refusal: Arc::clone(&self.refusal),
            attempts: Arc::clone(&self.attempts),
        })
    }
}

impl LiveWorkflowSessionFactory for RefusingFactory {
    fn capabilities(&self) -> serde_json::Value {
        self.inner.capabilities()
    }

    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.inner.target_catalog(actor)
    }

    fn open(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.inner
            .open(request, actor, definition, digest)
            .map(|inner| self.wrap(inner))
    }

    fn open_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition: &sts2_harness::workflow::WorkflowDefinition,
        digest: &str,
        limits: Option<&ContextOwnerControlLimits>,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        self.inner
            .open_admitted(request, actor, definition, digest, limits)
            .map(|inner| self.wrap(inner))
    }
}

struct RefusingSession {
    inner: Box<dyn LiveWorkflowSession>,
    refusal: Arc<Mutex<Option<ManagementError>>>,
    attempts: Arc<AtomicUsize>,
}

impl LiveWorkflowSession for RefusingSession {
    fn launch(&mut self) -> Result<(), ManagementError> {
        self.inner.launch()
    }
    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError> {
        self.inner.observe()
    }
    fn observe_projection(
        &mut self,
        projection: &str,
    ) -> Result<EpisodeObservation, ManagementError> {
        self.inner.observe_projection(projection)
    }
    fn legal_actions(
        &mut self,
        state: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, ManagementError> {
        self.inner.legal_actions(state, generation)
    }
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, ManagementError> {
        self.decide_for(input, "decision.live.v1", "context.live.v1")
    }
    fn decide_for(
        &mut self,
        input: &DecisionInput,
        profile: &str,
        context: &str,
    ) -> Result<Decision, ManagementError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let refusal = self.refusal.lock().expect("refusal control").clone();
        if let Some(error) = refusal {
            if matches!(
                error.class,
                ErrorClass::Unresolved | ErrorClass::WriteStateUnknown
            ) {
                // These cases deliberately cross the recording exchange before its reply is lost.
                self.inner.decide_for(input, profile, context)?;
            }
            return Err(error);
        }
        self.inner.decide_for(input, profile, context)
    }
    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError> {
        self.inner.dispatch_action(identity, action)
    }
    fn wait_for_transition(
        &mut self,
        operation: &str,
        wait: u32,
    ) -> Result<WaitSample, ManagementError> {
        self.inner.wait_for_transition(operation, wait)
    }
    fn reconcile(&mut self, operation: &str) -> Result<TransitionReceipt, ManagementError> {
        self.inner.reconcile(operation)
    }
    fn release_lease(&mut self) -> Result<(), ManagementError> {
        self.inner.release_lease()
    }
    fn stop_episode(&mut self) -> Result<(), ManagementError> {
        self.inner.stop_episode()
    }
}

#[test]
fn clean_context_pause_holds_the_cursor_and_clears_durable_intent_without_cleanup() {
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/context-pause-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&directory).expect("isolated database directory");
    let path = directory.join("workflow.sqlite3");
    let store = Arc::new(SqliteWorkflowStore::open(&path).expect("SQLite workflow store"));
    let factory = RefusingFactory::new(ErrorClass::Conflict);
    let service = live_service(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("live coordinator");
    let run = service
        .submit_run(&actor(), request("owner-pause", definition(false)))
        .expect("submit")
        .workflow_run_id;
    service
        .command(&actor(), command(&run, "observe", 1, CommandKind::Step))
        .expect("observe");
    let before = service
        .status(&actor(), &run)
        .expect("pre-pause cursor")
        .run;
    let first_request = command(&run, "held-step", 2, CommandKind::Step);
    let first = service
        .command(&actor(), first_request.clone())
        .expect("held step");
    assert_eq!(first.outcome, CommandOutcome::Applied);
    assert_eq!(
        service
            .command(&actor(), first_request.clone())
            .expect("exact retry"),
        first
    );
    assert_eq!(
        factory.attempts.load(Ordering::SeqCst),
        1,
        "duplicate does not prepare again"
    );
    let repeated = service
        .command(
            &actor(),
            command(&run, "held-step-again", 3, CommandKind::Step),
        )
        .expect("fresh command still respects owner pause");
    let held = service.status(&actor(), &run).expect("held cursor").run;
    assert_eq!(held.status, WorkflowRunStatus::Running);
    assert_eq!(held.cursor, before.cursor);
    assert_eq!(held.budget, before.budget);
    assert_eq!(held.cleanup, CleanupState::NotStarted);
    assert!(
        held.pending_operation.is_none(),
        "clean refusal clears the recorded intent"
    );
    assert_eq!(factory.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        factory.inner.entries(),
        ["launch", "observe", "legal_actions", "legal_actions"]
    );
    let events = service
        .events(&actor(), &run, 0, 128)
        .expect("durable events")
        .events;
    for sequence in [first.sequence, repeated.sequence] {
        let event = events
            .iter()
            .find(|event| Some(event.sequence) == sequence)
            .expect("step event");
        assert_eq!(
            event.payload.classification,
            Some(EventClassification::Rejected)
        );
        assert_eq!(event.payload.reason_code, CONTEXT_OWNER_PAUSED);
    }

    // The synthetic session now reports an explicitly opened owner gate. Production Resume is
    // exercised separately against the real owner; the coordinator cannot clear that gate itself.
    *factory.refusal.lock().expect("explicit gate transition") = None;
    service
        .command(
            &actor(),
            command(&run, "after-owner-resume", 4, CommandKind::Step),
        )
        .expect("fresh decision after explicit gate transition");
    let resumed = service.status(&actor(), &run).expect("resumed cursor").run;
    assert_ne!(resumed.cursor, before.cursor);
    assert_eq!(resumed.budget.provider_calls_consumed, 1);
    assert_eq!(
        resumed.budget.node_steps_consumed,
        before.budget.node_steps_consumed + 1
    );
    assert!(resumed.pending_operation.is_none());
    assert_eq!(
        factory
            .inner
            .entries()
            .iter()
            .filter(|entry| *entry == "decide")
            .count(),
        1
    );
    service
        .command(&actor(), command(&run, "cancel", 5, CommandKind::Cancel))
        .expect("cancel retained session");
    drop(service);
    assert_eq!(Arc::strong_count(&store), 1);
    drop(store);

    let restarted_factory = RefusingFactory::new(ErrorClass::Conflict);
    let reopened = Arc::new(SqliteWorkflowStore::open(&path).expect("reopen SQLite"));
    let restarted = live_service(
        reopened as Arc<dyn WorkflowStore>,
        Arc::clone(&restarted_factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("restarted coordinator");
    assert_eq!(
        restarted
            .command(&actor(), first_request)
            .expect("recover held response"),
        first
    );
    assert!(restarted_factory.inner.entries().is_empty());
    assert_eq!(restarted_factory.attempts.load(Ordering::SeqCst), 0);
    drop(restarted);
    std::fs::remove_dir_all(directory).expect("remove closed test database");
}

#[test]
fn the_same_pause_reason_after_exchange_retains_the_original_paid_attempt() {
    for class in [ErrorClass::Unresolved, ErrorClass::WriteStateUnknown] {
        let factory = RefusingFactory::new(class.clone());
        let service = live_service(
            Arc::new(MemoryWorkflowStore::new()),
            Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
            LiveWorkflowOptions::default(),
        )
        .expect("live coordinator");
        let run = service
            .submit_run(&actor(), request("post-exchange-pause", definition(false)))
            .expect("submit")
            .workflow_run_id;
        service
            .command(&actor(), command(&run, "observe", 1, CommandKind::Step))
            .expect("observe");
        let response = service
            .command(
                &actor(),
                command(&run, "paid-attempt", 2, CommandKind::Step),
            )
            .expect("unresolved exchange");
        assert_eq!(response.outcome, CommandOutcome::Pending, "{class:?}");
        let held = service
            .status(&actor(), &run)
            .expect("held paid attempt")
            .run;
        assert_eq!(held.status, WorkflowRunStatus::NeedsOperator);
        assert!(held.pending_operation.is_some());
        *factory.refusal.lock().expect("later gate transition") = None;
        service
            .command(
                &actor(),
                command(&run, "cannot-repay", 3, CommandKind::Step),
            )
            .expect("reconcile original attempt only");
        let after = service.status(&actor(), &run).expect("still held").run;
        assert_eq!(after.pending_operation, held.pending_operation);
        assert_eq!(factory.attempts.load(Ordering::SeqCst), 1, "{class:?}");
        assert_eq!(
            factory
                .inner
                .entries()
                .iter()
                .filter(|entry| *entry == "decide")
                .count(),
            1
        );
    }
}

#[test]
fn a_pause_code_without_the_clean_conflict_class_never_becomes_a_resumable_hold() {
    for class in [
        ErrorClass::Unavailable,
        ErrorClass::Forbidden,
        ErrorClass::Capability,
    ] {
        let factory = RefusingFactory::new(class.clone());
        let service = live_service(
            Arc::new(MemoryWorkflowStore::new()),
            Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
            LiveWorkflowOptions::default(),
        )
        .expect("live coordinator");
        let run = service
            .submit_run(&actor(), request("not-a-pause", definition(false)))
            .expect("submit")
            .workflow_run_id;
        service
            .command(&actor(), command(&run, "observe", 1, CommandKind::Step))
            .expect("observe");
        service
            .command(&actor(), command(&run, "refused", 2, CommandKind::Step))
            .expect("terminal refusal");
        let refused = service.status(&actor(), &run).expect("failed cursor").run;
        assert_eq!(refused.status, WorkflowRunStatus::Failed, "{class:?}");
        assert_eq!(refused.cleanup, CleanupState::Complete);
        assert!(refused.pending_operation.is_none());
        assert!(
            !factory
                .inner
                .entries()
                .iter()
                .any(|entry| entry == "decide")
        );
    }
}
