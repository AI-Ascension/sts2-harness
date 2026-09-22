// SPDX-License-Identifier: MIT

//! Served acceptance for the durable prepared-dispatch receipt ledger.
//!
//! `#108` AC4 requires that a restarted served composition refuses to write an application boundary
//! its predecessor already accepted. Until the receipts could be carried across a process restart,
//! the once-only guarantee rested on the in-memory ledger of one served session, so a restart
//! re-derived an empty ledger and wrote the same approved bytes a second time. These tests drive the
//! real [`ServedBoundaryLedger`] the served composition opens around an owner-supplied
//! `DispatchLedgerPort`, so a restart that loses the durable image is a failure rather than a
//! missing assertion.

use super::super::tests::Counters;
use super::boundary_tests::{ObservingSink, SELECTED_ITEMS};
use super::managed_render_tests::{
    render_fixture, render_test_session, render_test_session_with_state, selected_limits,
};
use super::*;
use crate::context_capture::{
    DispatchLedger, DispatchLedgerError, DispatchLedgerPort, DispatchOutcome,
    DurableDispatchLedger, InMemoryDispatchLedgerPort,
};
use crate::episode::RuntimeLeaseBinding;
use crate::management::{ErrorClass, LiveWorkflowSessionFactory};
use crate::{
    ExoConfig, ExoDecisionSource, ExoProvider, ExoSession, ExoTransport, ExoTransportError,
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The single model execution the render fixture decides for.
const EXECUTION_ID: &str = "model-execution-1";

/// The dispatch identity the served boundary derives for the fixture execution.
fn dispatch_id() -> String {
    format!("exo.{}", &crate::sha256_hex(EXECUTION_ID.as_bytes())[..32])
}

/// A durable ledger port whose load and save behaviour is scripted, so a served session can be
/// pointed at a store that is missing, unreachable, inconsistent or unwritable.
#[derive(Debug)]
struct ScriptedLedgerPort {
    loaded: Result<Option<DurableDispatchLedger>, DispatchLedgerError>,
    save_error: Option<DispatchLedgerError>,
}

impl DispatchLedgerPort for ScriptedLedgerPort {
    fn load(&mut self) -> Result<Option<DurableDispatchLedger>, DispatchLedgerError> {
        self.loaded.clone()
    }

    fn save(&mut self, _ledger: &DurableDispatchLedger) -> Result<(), DispatchLedgerError> {
        match self.save_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// A transport that never returns a reply, so the release reports an indeterminate outcome.
struct TimingOutTransport {
    attempts: Arc<AtomicUsize>,
}

impl ExoTransport for TimingOutTransport {
    fn exchange(
        &mut self,
        _request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(ExoTransportError::Timeout)
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

/// Builds the served session fixture with the recording sink the served composition attaches.
fn served_session() -> (
    ProductionLiveWorkflowSession,
    Arc<Mutex<super::managed_render_tests::RenderState>>,
    Arc<AtomicUsize>,
    ExoConfig,
) {
    let limits = selected_limits(SELECTED_ITEMS);
    let (source, config) = render_fixture();
    let (mut session, state, exchanges, _) =
        render_test_session(source, config.clone(), limits, Default::default(), None);
    session.boundary_capture = BoundaryCaptureSink::new(Box::new(ObservingSink::new()));
    (session, state, exchanges, config)
}

/// The served factory and the counters its runtime fixtures write to, so a refusal can be shown to
/// happen before the runtime is opened.
fn served_factory(
    port: Box<dyn DispatchLedgerPort>,
) -> (
    ProductionLiveWorkflowSessionFactory,
    Arc<Mutex<Counters>>,
    RunRequest,
    AuthContext,
    WorkflowDefinition,
    String,
) {
    use super::super::tests::{Catalog, Policy, Provider, RuntimeFactory};

    let definition: WorkflowDefinition = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("workflow definition");
    let digest = "d".repeat(64);
    let request = RunRequest {
        schema_version: crate::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "boundary-durable-test".to_owned(),
        definition: None,
        artifact_id: None,
        instance_id: "test-instance".to_owned(),
        profile: crate::management::LIVE_WORKFLOW_PROFILE.to_owned(),
        admission: None,
    };
    let run_id = live_run_id(&request, &digest).expect("run identity");
    let authority = RuntimeAuthorityBinding {
        instance_id: request.instance_id.clone(),
        session_id: "runtime-session".to_owned(),
        lease_id: "configured-lease".to_owned(),
        lease_epoch: 1,
        run_id: run_id.clone(),
        episode_id: "test-episode".to_owned(),
        trajectory_id: "test-trajectory".to_owned(),
        trace_id: "test-trace".to_owned(),
        artifact_id: "test-artifact".to_owned(),
        agent_id: "test-agent".to_owned(),
        adapter_revision: "test-adapter".to_owned(),
        model_revision: "test-model".to_owned(),
        configuration_digest: "b".repeat(64),
        output_schema_digest: "c".repeat(64),
    };
    let acquired = RuntimeLeaseBinding {
        instance_id: request.instance_id.clone(),
        session_id: "runtime-session".to_owned(),
        run_id,
        lease_id: "gateway-recovery-lease".to_owned(),
        lease_epoch: 3,
    };
    let observation = EpisodeObservation::new(
        "combat-1",
        1,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1",
            "generation":1,
            "visible_seed":"fixture",
            "player":{"hp":1,"max_hp":1,"energy":1,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("observation");
    let counters = Arc::new(Mutex::new(Counters::default()));
    let factory = ProductionLiveWorkflowSessionFactory::new(
        serde_json::json!({"capabilities":[]}),
        Arc::new(Catalog),
        Arc::new(RuntimeFactory {
            authority,
            acquired,
            observation,
            counters: Arc::clone(&counters),
        }),
        Arc::new(Provider {
            counters: Arc::clone(&counters),
        }),
        Arc::new(Policy),
        NativeCapabilities::fixture(),
    )
    .expect("production factory")
    .with_dispatch_ledger_port(port);
    let actor = AuthContext::new("test-actor", ["workflow:*".to_owned()]).expect("actor");
    (factory, counters, request, actor, definition, digest)
}

#[test]
fn served_factory_refuses_an_unreadable_ledger_before_opening_the_runtime() {
    let (factory, counters, request, actor, definition, digest) =
        served_factory(Box::new(ScriptedLedgerPort {
            loaded: Err(DispatchLedgerError::Unavailable),
            save_error: None,
        }));

    let error = factory
        .open_admitted(&request, &actor, &definition, &digest, None)
        .map(drop)
        .expect_err("the served composition refuses a store it cannot read");

    assert_eq!(error.code, "prepared_boundary_ledger_unavailable");
    assert_eq!(error.class, ErrorClass::Unavailable);
    assert_eq!(
        counters.lock().expect("counter lock").runtime_opens,
        0,
        "a refused durable store must not leak an opened runtime"
    );
}

#[test]
fn served_boundary_restart_reloads_the_committed_receipt_and_refuses_a_second_write() {
    let (mut first, state, first_exchanges, config) = served_session();
    let limits = selected_limits(SELECTED_ITEMS);
    let store = ServedDispatchLedger::new(Box::new(InMemoryDispatchLedgerPort::default()));
    let id = dispatch_id();

    first.boundary = ServedBoundaryLedger::restored(store.clone())
        .expect("a store with no image restores an empty ledger");
    first
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the first served decision is admitted");
    assert_eq!(first_exchanges.load(Ordering::SeqCst), 1);
    assert!(first.boundary.receipt(&id).is_some());
    drop(first);

    // The store carries the receipt the first session committed, so the restarted composition
    // rebuilds its ledger from that image rather than from an empty one.
    let committed = store
        .load()
        .expect("the in-memory store is readable")
        .expect("the first session persisted its image");
    assert_eq!(committed.receipts.len(), 1);
    assert_eq!(committed.receipts[0].dispatch_id, id);

    let (mut second, _, second_exchanges, _) = render_test_session_with_state(
        Arc::clone(&state),
        config,
        limits,
        Default::default(),
        None,
    );
    let second_sink = ObservingSink::new();
    second.boundary_capture = BoundaryCaptureSink::new(Box::new(second_sink.clone()));
    second.boundary = ServedBoundaryLedger::restored(store.clone())
        .expect("the restarted session reloads the committed receipt");

    let error = second
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("the reloaded receipt forbids a second write of the same approval");

    assert_eq!(error.code, "prepared_boundary_already_recorded");
    assert_eq!(error.class, ErrorClass::Unresolved);
    assert_eq!(second_exchanges.load(Ordering::SeqCst), 0);
    assert_eq!(second_sink.depth(), (0, 0));
}

#[test]
fn served_boundary_restart_without_a_durable_port_writes_the_approval_again() {
    // The same restart as above, but each session keeps its own session-lifetime ledger, which is
    // what a composition that attaches no durable port gets. The second write proves the refusal
    // above is the restored receipt and not the fixture.
    let (mut first, state, _, config) = served_session();
    first
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the first served decision is admitted");
    assert!(first.boundary.receipt(&dispatch_id()).is_some());
    drop(first);

    let (mut second, _, second_exchanges, _) = render_test_session_with_state(
        Arc::clone(&state),
        config,
        selected_limits(SELECTED_ITEMS),
        Default::default(),
        None,
    );
    second
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("without a durable port the restarted session writes the approval again");
    assert_eq!(second_exchanges.load(Ordering::SeqCst), 1);
    assert!(second.boundary.receipt(&dispatch_id()).is_some());
}

#[test]
fn served_boundary_refuses_a_store_it_cannot_read() {
    let store = ServedDispatchLedger::new(Box::new(ScriptedLedgerPort {
        loaded: Err(DispatchLedgerError::Unavailable),
        save_error: None,
    }));

    let error = ServedBoundaryLedger::restored(store)
        .expect_err("an unreadable store leaves the recorded receipts unknown");

    assert_eq!(error.code, "prepared_boundary_ledger_unavailable");
    assert_eq!(error.class, ErrorClass::Unavailable);
}

#[test]
fn served_boundary_does_not_persist_a_refusal_that_preceded_the_write() {
    let (mut session, _, exchanges, _) = served_session();
    session.boundary_capture = BoundaryCaptureSink::disabled();
    let store = ServedDispatchLedger::new(Box::new(InMemoryDispatchLedgerPort::default()));
    session.boundary = ServedBoundaryLedger::restored(store.clone())
        .expect("a store with no image restores an empty ledger");

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a sink that cannot record refuses before the write");

    // Nothing reached the provider, so no receipt is retained: a durable "already recorded" mark
    // here would make the decision permanently unretryable across a restart.
    assert_eq!(error.code, "prepared_boundary_unsupported");
    assert_eq!(error.class, ErrorClass::Capability);
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
    assert!(session.boundary.receipt(&dispatch_id()).is_none());
    assert_eq!(store.load().expect("the store is readable"), None);
}

#[test]
fn served_boundary_refuses_a_store_whose_image_is_inconsistent() {
    let mut image = DispatchLedger::new().durable();
    image.schema = "ascension.dispatch-ledger.v2".to_owned();
    let store = ServedDispatchLedger::new(Box::new(ScriptedLedgerPort {
        loaded: Ok(Some(image)),
        save_error: None,
    }));

    let error =
        ServedBoundaryLedger::restored(store).expect_err("an image that cannot be trusted refuses");

    assert_eq!(error.code, "prepared_boundary_ledger_inconsistent");
    assert_eq!(error.class, ErrorClass::Unavailable);
}

#[test]
fn served_boundary_reports_an_unresolved_outcome_when_the_store_cannot_be_written() {
    let (mut session, _, exchanges, _) = served_session();
    session.boundary =
        ServedBoundaryLedger::restored(ServedDispatchLedger::new(Box::new(ScriptedLedgerPort {
            loaded: Ok(None),
            save_error: Some(DispatchLedgerError::Unavailable),
        })))
        .expect("an empty readable store restores an empty ledger");

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a store that cannot be written leaves the recorded receipt unresolved");

    // The exchange already happened inside the release, so this is an unresolved outcome rather
    // than a clean refusal, and the receipt is still retained for the running session.
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(error.code, "prepared_boundary_ledger_unavailable");
    assert_eq!(error.class, ErrorClass::Unresolved);
    assert!(session.boundary.receipt(&dispatch_id()).is_some());
}

#[test]
fn served_boundary_restart_refuses_again_after_an_indeterminate_write() {
    let (mut first, state, _, config) = served_session();
    let attempts = Arc::new(AtomicUsize::new(0));
    // A transport that never replies is exactly the lost-reply case the durable image must keep:
    // `resume` records the receipt and the served session persists it before reporting failure.
    first.provider = Some(Box::new(ExoDecisionSource::new(ExoSession::new(
        ExoProvider::new(
            TimingOutTransport {
                attempts: Arc::clone(&attempts),
            },
            config.clone(),
        ),
    ))));
    let store = ServedDispatchLedger::new(Box::new(InMemoryDispatchLedgerPort::default()));
    first.boundary = ServedBoundaryLedger::restored(store.clone())
        .expect("a store with no image restores an empty ledger");

    let error = first
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a transport that never replies is an indeterminate outcome");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(error.code, "provider_decision_failed");
    assert_eq!(error.class, ErrorClass::Unresolved);

    let retained = first
        .boundary
        .receipt(&dispatch_id())
        .expect("an indeterminate write still records its receipt");
    assert_eq!(retained.outcome, DispatchOutcome::Unknown);
    assert_eq!(retained.written_bytes, 0);
    assert_eq!(retained.gameplay_effects, 0);
    drop(first);

    let committed = store
        .load()
        .expect("the in-memory store is readable")
        .expect("the indeterminate receipt is persisted");
    assert_eq!(committed.receipts.len(), 1);
    assert_eq!(committed.receipts[0].outcome, DispatchOutcome::Unknown);

    let (mut second, _, second_exchanges, _) = render_test_session_with_state(
        Arc::clone(&state),
        config,
        selected_limits(SELECTED_ITEMS),
        Default::default(),
        None,
    );
    second.boundary = ServedBoundaryLedger::restored(store)
        .expect("the restarted session reloads the indeterminate receipt");

    let error = second
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a reloaded indeterminate receipt still forbids a second write");
    assert_eq!(error.code, "prepared_boundary_already_recorded");
    assert_eq!(second_exchanges.load(Ordering::SeqCst), 0);
}
