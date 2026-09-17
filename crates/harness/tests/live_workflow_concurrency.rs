// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use sts2_harness::management::{
    AuthContext, CommandKind, CommandOutcome, CommandRequest, CommandResponse, LiveWorkflowOptions,
    LiveWorkflowSession, LiveWorkflowSessionFactory, ManagementError, MemoryWorkflowStore,
    RunRequest, TargetCatalogResponse, WorkflowRunStatus,
};
use sts2_harness::{
    ActionIdentity, Decision, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, TransitionReceipt, WaitOutcome, WaitSample,
};

#[path = "support/live_workflow.rs"]
mod support;

#[path = "support/live_workflow_concurrency_factory.rs"]
mod concurrency_factory;

use concurrency_factory::GatedFactory;
use support::*;

#[path = "live_workflow_concurrency/cancellation.rs"]
mod cancellation;

/// A one-shot coordination gate that pins a wrapped live session inside a
/// chosen operation. It lets two command threads be interleaved
/// deterministically so a scheduler race can be reproduced rather than only
/// described.
struct SessionGate {
    target: String,
    consumed: Mutex<bool>,
    entered: Mutex<bool>,
    entered_cv: Condvar,
    release: Mutex<bool>,
    release_cv: Condvar,
}

impl SessionGate {
    fn new(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            consumed: Mutex::new(false),
            entered: Mutex::new(false),
            entered_cv: Condvar::new(),
            release: Mutex::new(false),
            release_cv: Condvar::new(),
        }
    }

    fn block_if(&self, operation: &str) {
        if operation != self.target {
            return;
        }
        let mut consumed = self.consumed.lock().expect("gate consumed");
        if *consumed {
            return;
        }
        *consumed = true;
        drop(consumed);
        let mut entered = self.entered.lock().expect("gate entered");
        *entered = true;
        self.entered_cv.notify_all();
        drop(entered);
        let mut release = self.release.lock().expect("gate release");
        while !*release {
            release = self.release_cv.wait(release).expect("gate wait");
        }
    }

    fn wait_entered(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut entered = self.entered.lock().expect("gate entered");
        while !*entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (guard, result) = self
                .entered_cv
                .wait_timeout(entered, remaining)
                .expect("gate wait");
            entered = guard;
            if result.timed_out() && !*entered {
                return false;
            }
        }
        true
    }

    fn release(&self) {
        let mut release = self.release.lock().expect("gate release");
        *release = true;
        self.release_cv.notify_all();
    }
}

/// Releases the gate when a test leaves the protected region, including on a
/// failed assertion, so a gated worker can never be stranded.
struct GateRelease(Arc<SessionGate>);

impl Drop for GateRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn command_with_timeout(
    service: &Arc<sts2_harness::management::ManagementService>,
    actor: &AuthContext,
    request: CommandRequest,
    timeout: Duration,
) -> Result<CommandResponse, ManagementError> {
    let (sender, receiver) = mpsc::channel();
    let service = Arc::clone(service);
    let actor = actor.clone();
    thread::spawn(move || {
        let _ = sender.send(service.command(&actor, request));
    });
    receiver
        .recv_timeout(timeout)
        .expect("command did not return before the bounded deadline")
}

struct GatedSession {
    inner: Box<dyn LiveWorkflowSession>,
    gate: Arc<SessionGate>,
    timeout_after_wait_gate: bool,
}

impl LiveWorkflowSession for GatedSession {
    fn launch(&mut self) -> Result<(), ManagementError> {
        self.inner.launch()
    }

    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError> {
        self.gate.block_if("observe");
        self.inner.observe()
    }

    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, ManagementError> {
        self.gate.block_if("observe");
        self.inner.observe_projection(projection_ref)
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, ManagementError> {
        self.inner.legal_actions(state_id, generation)
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, ManagementError> {
        self.inner.decide(input)
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<Decision, ManagementError> {
        self.inner
            .decide_for(input, decision_profile_ref, context_ref)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError> {
        self.gate.block_if("dispatch");
        self.inner.dispatch_action(identity, action)
    }

    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, ManagementError> {
        self.gate.block_if("wait");
        if self.timeout_after_wait_gate {
            return Ok(WaitSample::new(WaitOutcome::Timeout, None));
        }
        self.inner
            .wait_for_transition(operation_id, wait_for_millis)
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, ManagementError> {
        self.inner.reconcile(operation_id)
    }

    fn release_lease(&mut self) -> Result<(), ManagementError> {
        self.inner.release_lease()
    }

    fn stop_episode(&mut self) -> Result<(), ManagementError> {
        self.inner.stop_episode()
    }

    fn pause(&mut self) -> Result<(), ManagementError> {
        self.inner.pause()
    }

    fn resume(&mut self) -> Result<(), ManagementError> {
        self.inner.resume()
    }

    fn action_completed(&mut self, settled: bool) {
        self.inner.action_completed(settled);
    }

    fn model_execution_id(&self) -> Option<sts2_harness::ModelExecutionId> {
        self.inner.model_execution_id()
    }
}

fn gated_service(
    gate: &Arc<SessionGate>,
    timeout_after_wait_gate: bool,
) -> (
    Arc<sts2_harness::management::ManagementService>,
    Arc<FakeFactory>,
) {
    let inner = Arc::new(FakeFactory::new(false));
    let factory = Arc::new(GatedFactory {
        inner: Arc::clone(&inner),
        gate: Arc::clone(gate),
        timeout_after_wait_gate,
    });
    let service = live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    (Arc::new(service), inner)
}

#[test]
fn concurrent_command_at_the_same_revision_is_deferred_and_applied_once() {
    let gate = Arc::new(SessionGate::new("observe"));
    let (service, factory) = gated_service(&gate, false);
    let actor = actor();
    let submitted = service
        .submit_run(
            &actor,
            request("request-live-race-revision", definition(false)),
        )
        .expect("submit");
    let run_id = submitted.workflow_run_id;

    // Thread A accepts the observe step and stalls inside the live session.
    let racing = {
        let service = Arc::clone(&service);
        let actor = actor.clone();
        let run_id = run_id.clone();
        thread::spawn(move || {
            service.command(&actor, command(&run_id, "race-first", 1, CommandKind::Step))
        })
    };
    let _release = GateRelease(Arc::clone(&gate));
    assert!(
        gate.wait_entered(Duration::from_secs(10)),
        "worker did not reach the gated observe port"
    );

    // A distinct command at the same expected revision cannot double-apply; it
    // is deferred (not queued) while the first command is durably in flight.
    let deferred = command_with_timeout(
        &service,
        &actor,
        command(&run_id, "race-second", 1, CommandKind::Step),
        Duration::from_secs(10),
    )
    .expect("concurrent command");
    assert_eq!(deferred.outcome, CommandOutcome::Pending);
    assert_eq!(deferred.run_revision, 1);
    assert_eq!(deferred.sequence, None);

    gate.release();
    let applied = racing.join().expect("join").expect("first command");
    assert_eq!(applied.outcome, CommandOutcome::Applied);
    assert_eq!(applied.run_revision, 2);

    // Exactly one node executed and the durable revision advanced once.
    let snapshot = service.status(&actor, &run_id).expect("status").run;
    assert_eq!(snapshot.run_revision, 2);
    assert_eq!(snapshot.status, WorkflowRunStatus::Running);
    assert_eq!(
        factory
            .entries()
            .iter()
            .filter(|entry| entry.as_str() == "observe")
            .count(),
        1
    );
}
