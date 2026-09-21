// SPDX-License-Identifier: MIT

//! `#94`: the live-submission teardown fences.
//!
//! A submission that opened a session and was then refused has to tear that session down before it
//! returns, and so does a submission whose launch failed. When that teardown *also* fails, the
//! failure must reach the operator — `live_launch_cleanup_failed` / `live_duplicate_cleanup_failed`
//! — instead of being discarded and leaving a launched episode behind while the caller is told
//! only "the launch failed" or "the identity was taken".
//!
//! [`super::duplicate_run_tests`] pins the refusals that happen *before* a session exists. This
//! module pins the two branches where a session exists and cannot be stopped, plus the control that
//! shows the same launch fault is reported as the plain launch error when the teardown succeeds.
//! The composition is the real assembly [`live_port`] builds; only the gateway/MCP runtime and the
//! provider are fixtures, and the teardown failure is armed on the fixture counters rather than by
//! changing production code.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use super::Counters;
use super::duplicate_run_tests::{
    CountingStore, admitted_request, counts, live_port, submit, unused,
};
use crate::management::store::SubmissionLookup;
use crate::management::{
    AuthContext, CommandAcceptance, CommandRequest, CommandResponse, ErrorClass, EventPage,
    LiveWorkflowExecutionPort, ManagementError, RunAdmission, RunEvent, RunRequest, RunSnapshot,
    StoreError, WorkflowRunStatus, WorkflowStore, digest_value, live_run_id,
};

/// A store that lands the competing admission at the reservation step.
///
/// `create_run` is a submission's last step before it opens its session, so completing a competing
/// admission from inside it puts that admission in the registry *after* this submission's early
/// identity check and *before* its registry insert — the interleaving the registry-lock re-check
/// exists for. The competing admission is admitted in full rather than the window being widened by
/// a sleep.
struct RacingStore {
    landing: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl RacingStore {
    fn new(landing: Box<dyn FnOnce() + Send>) -> Self {
        Self {
            landing: Mutex::new(Some(landing)),
        }
    }
}

impl WorkflowStore for RacingStore {
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
        if let Some(landing) = self.landing.lock().expect("landing lock").take() {
            landing();
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
        _application: crate::management::store::CommandApplication,
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

    fn export(
        &self,
        _run_id: &str,
        _redacted: bool,
    ) -> Result<crate::management::ExportResponse, StoreError> {
        Err(unused("export"))
    }
}

/// The admitted request, its digests, its identity and an actor, ready to submit.
struct Submission {
    request: RunRequest,
    definition_digest: String,
    request_digest: String,
    run_id: String,
    actor: AuthContext,
}

fn submission() -> Submission {
    let (request, definition_digest) = admitted_request();
    let request_digest = digest_value(&serde_json::to_value(&request).expect("request encode"))
        .expect("request digest");
    let run_id = live_run_id(&request, &definition_digest).expect("run identity");
    let actor = AuthContext::new("cleanup-fence-actor", ["workflow:*".to_owned()]).expect("actor");
    Submission {
        request,
        definition_digest,
        request_digest,
        run_id,
        actor,
    }
}

impl Submission {
    fn send(
        &self,
        port: &LiveWorkflowExecutionPort,
        store: Arc<dyn WorkflowStore>,
    ) -> Result<RunAdmission, ManagementError> {
        submit(
            port,
            &self.request,
            &self.actor,
            &self.definition_digest,
            &self.request_digest,
            store,
        )
    }
}

/// Counters armed with teardown faults for one submission.
fn armed_counters(arm: impl FnOnce(&mut Counters)) -> Arc<Mutex<Counters>> {
    let mut counters = Counters::default();
    arm(&mut counters);
    Arc::new(Mutex::new(counters))
}

#[test]
fn live_launch_cleanup_failed_is_reported_when_the_launched_session_cannot_be_stopped() {
    let submission = submission();
    let counters = armed_counters(|counters| {
        counters.launch_fault_on_call = Some(1);
        counters.stop_fault_on_call = Some(1);
    });
    let port = live_port(&counters, &submission.run_id);

    let error = submission
        .send(&port, Arc::new(CountingStore::new()))
        .expect_err("a launch failure must refuse the submission");
    assert_eq!(error.code, "live_launch_cleanup_failed");
    assert_eq!(error.class, ErrorClass::Unavailable);
    assert!(
        error.message.contains("test_launch_failure"),
        "the launch failure the caller has to reason about is preserved: {}",
        error.message
    );
    assert!(
        error.message.contains("recovery port failed"),
        "the teardown failure that could not be completed is preserved: {}",
        error.message
    );
    assert_eq!(
        counts(&counters),
        (1, 0, 0, 0),
        "the launch failed before the provider was opened, so no provider session exists to leave behind"
    );
    let observed = counters.lock().expect("counter lock");
    assert_eq!(observed.launch_calls, 1);
    assert_eq!(
        observed.stop_calls, 1,
        "the refusal attempted the teardown of the session it had launched"
    );
}

#[test]
fn a_launch_failure_whose_teardown_succeeds_is_reported_as_the_launch_error() {
    // The control for the test above: the same launch fault, with a teardown that works, must not
    // be reported as a cleanup failure. If the cleanup reporting were removed, the test above would
    // see this code and fail; if it were reported unconditionally, this test would fail.
    let submission = submission();
    let counters = armed_counters(|counters| {
        counters.launch_fault_on_call = Some(1);
    });
    let port = live_port(&counters, &submission.run_id);

    let error = submission
        .send(&port, Arc::new(CountingStore::new()))
        .expect_err("a launch failure must refuse the submission");
    assert_eq!(error.code, "live_launch_failed");
    assert_eq!(error.class, ErrorClass::Unavailable);
    assert!(
        error.message.contains("test_launch_failure"),
        "the original launch failure is the reported one: {}",
        error.message
    );
    assert!(
        !error.message.contains("cleanup failed"),
        "a teardown that succeeded is not reported as a failure: {}",
        error.message
    );
    assert_eq!(
        counters.lock().expect("counter lock").stop_calls,
        1,
        "the refusal still attempted the teardown"
    );
    assert_eq!(
        counts(&counters),
        (1, 0, 0, 0),
        "a refused submission leaves no second runtime, provider session or dispatch"
    );

    // Nothing was registered for the failed submission, so the identity is still free: the fence
    // reports a refusal without consuming the identity it refused.
    let admitted = submission
        .send(&port, Arc::new(CountingStore::new()))
        .expect("a submission that left no effect must not hold its identity");
    assert_eq!(admitted.snapshot.workflow_run_id, submission.run_id);
    assert_eq!(admitted.snapshot.status, WorkflowRunStatus::Running);
}

#[test]
fn live_duplicate_cleanup_failed_is_reported_when_a_racing_duplicate_cannot_be_stopped() {
    let submission = submission();
    // The refused submission launched a session and cannot stop it, so the teardown the refusal
    // performs is the failure that has to reach the operator.
    let counters = armed_counters(|counters| {
        counters.stop_fault_on_call = Some(1);
    });
    let port = Arc::new(live_port(&counters, &submission.run_id));

    let winner: Arc<Mutex<Option<Result<RunAdmission, ManagementError>>>> =
        Arc::new(Mutex::new(None));
    let landing = {
        let port = Arc::clone(&port);
        let winner = Arc::clone(&winner);
        let request = submission.request.clone();
        let actor = submission.actor.clone();
        let definition_digest = submission.definition_digest.clone();
        let request_digest = submission.request_digest.clone();
        Box::new(move || {
            let store: Arc<dyn WorkflowStore> = Arc::new(CountingStore::new());
            let result = submit(
                &port,
                &request,
                &actor,
                &definition_digest,
                &request_digest,
                store,
            );
            *winner.lock().expect("winner lock") = Some(result);
        })
    };
    let store: Arc<dyn WorkflowStore> = Arc::new(RacingStore::new(landing));

    let error = submission
        .send(&port, store)
        .expect_err("a duplicate that raced past the early check must still be refused");
    assert_eq!(error.code, "live_duplicate_cleanup_failed");
    assert_eq!(error.class, ErrorClass::Unavailable);
    assert!(
        error.message.contains("live_stop_failed"),
        "the teardown failure is what makes this refusal operator-visible: {}",
        error.message
    );

    let winner = winner
        .lock()
        .expect("winner lock")
        .take()
        .expect("the competing admission landed at the reservation step")
        .expect("the competing admission is admitted");
    assert_eq!(winner.snapshot.workflow_run_id, submission.run_id);
    assert_eq!(
        winner.snapshot.status,
        WorkflowRunStatus::Running,
        "the admission that won the registry lock stays authoritative"
    );
    assert_eq!(
        counts(&counters),
        (2, 0, 0, 2),
        "the winner and the refused duplicate each opened one runtime and one provider session"
    );
    assert_eq!(
        counters.lock().expect("counter lock").stop_calls,
        1,
        "only the refused duplicate's session was torn down"
    );
}
