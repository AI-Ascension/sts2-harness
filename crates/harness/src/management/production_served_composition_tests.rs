// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Composition-level acceptance for the served factory the served binary builds.
//!
//! `render_test_session` fixtures assemble a session field-by-field, so they prove the boundary's
//! behaviour but not that the *composition* wires it. These tests open the real
//! `ProductionLiveWorkflowSessionFactory` -- the type `workflow_service.rs` configures -- and drive
//! the same `decide_for` seam, so `#398` AC1 ("the served composition attaches a recording sink so a
//! served managed decision completes") and AC2 ("a composition with no recording sink still refuses
//! before any provider write") are asserted against the composition rather than a hand-built session.

use super::boundary_durable_tests::dispatch_id;
use super::boundary_tests::{ObservingSink, SELECTED_ITEMS, expected_provider_bytes};
use super::managed_render_tests::{
    render_fixture, render_port, render_test_session, selected_limits,
};
use super::*;
use crate::context_capture::{
    ADVERTISED_EXACT_ADAPTERS, CaptureBoundary, CaptureComponentKind, TransportState,
    adapter_support,
};
use crate::episode::{EpisodeObservation, EpisodeStage, RuntimeLeaseBinding};
use crate::management::{
    AuthContext, ContextOwnerControlLimits, ErrorClass, LiveContextObservationPort,
    LiveProviderSessionFactory, LiveWorkflowSession, LiveWorkflowSessionFactory, ManagementError,
    RunRequest, RuntimeAuthorityBinding,
};
use crate::provider_session::NativeCapabilities;
use crate::workflow::WorkflowDefinition;
use crate::{
    ExoConfig, ExoDecisionSource, ExoProvider, ExoSession, ExoTransport, ExoTransportError,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A transport that records the managed request and answers with one bounded action.
struct ManagedRecordingTransport {
    exchanges: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ExoTransport for ManagedRecordingTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .map_err(|_| ExoTransportError::Unavailable)?
            .push(request.to_vec());
        Ok(
            br#"{"decision":"action","action_id":"combat.end-turn","rationale":"served composition decision"}"#
                .to_vec(),
        )
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

/// The provider session the served composition opens: a real Exo decision source over the recording
/// transport, so the managed decision can complete instead of being refused by the provider fixture.
struct ServedExoProviderFactory {
    config: ExoConfig,
    exchanges: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl LiveProviderSessionFactory for ServedExoProviderFactory {
    fn open_provider(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn DecisionSource + Send>, ManagementError> {
        let transport = ManagedRecordingTransport {
            exchanges: Arc::clone(&self.exchanges),
            requests: Arc::clone(&self.requests),
        };
        Ok(Box::new(ExoDecisionSource::new(ExoSession::new(
            ExoProvider::new(transport, self.config.clone()),
        ))))
    }
}

/// One served composition opened through the real production factory.
struct ServedComposition {
    factory: ProductionLiveWorkflowSessionFactory,
    request: RunRequest,
    actor: AuthContext,
    definition: WorkflowDefinition,
    digest: String,
    limits: ContextOwnerControlLimits,
    sink: Option<ObservingSink>,
    expected: Vec<u8>,
    exchanges: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServedComposition {
    /// Opens, launches and hands back the served session the composition builds.
    fn open(&self) -> Box<dyn LiveWorkflowSession> {
        let mut session = self
            .factory
            .open_admitted(
                &self.request,
                &self.actor,
                &self.definition,
                &self.digest,
                Some(&self.limits),
            )
            .expect("the served composition admits the run");
        session.launch().expect("the served composition launches");
        session
    }
}

/// Builds the served composition with or without the recording sink `workflow_service.rs` attaches.
fn served_composition(attach_recording_sink: bool) -> ServedComposition {
    use super::super::tests::{CapturingOwner, Catalog, Counters, Policy, RuntimeFactory};

    let definition: WorkflowDefinition = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("workflow definition");
    let digest = "d".repeat(64);
    let request = RunRequest {
        schema_version: crate::management::MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "served-composition-boundary-test".to_owned(),
        definition: None,
        artifact_id: None,
        instance_id: "test-instance".to_owned(),
        profile: crate::management::LIVE_WORKFLOW_PROFILE.to_owned(),
        admission: None,
    };
    let run_id = crate::management::live_run_id(&request, &digest).expect("run identity");
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

    let limits = selected_limits(SELECTED_ITEMS);
    let (source, config) = render_fixture();
    let expected = expected_provider_bytes(&source, &config, limits);
    let exchanges = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let counters = Arc::new(Mutex::new(Counters::default()));
    let sink = ObservingSink::new();
    let mut factory = ProductionLiveWorkflowSessionFactory::new(
        serde_json::json!({"capabilities":[]}),
        Arc::new(Catalog),
        Arc::new(RuntimeFactory {
            authority,
            acquired,
            observation,
            counters,
        }),
        Arc::new(ServedExoProviderFactory {
            config,
            exchanges: Arc::clone(&exchanges),
            requests: Arc::clone(&requests),
        }),
        Arc::new(Policy),
        NativeCapabilities::fixture(),
    )
    .expect("production factory")
    .with_context_observations(
        Arc::new(CapturingOwner::default()) as Arc<dyn LiveContextObservationPort>
    )
    .with_context_render_port(render_port(
        source,
        limits,
        Arc::new(AtomicBool::new(false)),
    ));
    let sink = if attach_recording_sink {
        factory = factory.with_capture_sink(BoundaryCaptureSink::new(Box::new(sink.clone())));
        Some(sink)
    } else {
        None
    };
    ServedComposition {
        factory,
        request,
        actor: AuthContext::new("test-actor", ["workflow:*".to_owned()]).expect("actor"),
        definition,
        digest,
        limits: ContextOwnerControlLimits {
            schema_version: crate::management::CONTEXT_OWNER_CONTROL_LIMITS_SCHEMA.to_owned(),
            owner_id: "test-owner".to_owned(),
            owner_version: "v1".to_owned(),
            catalog_digest: "e".repeat(64),
            max_control_events: 1,
        },
        sink,
        expected,
        exchanges,
        requests,
    }
}

#[test]
fn the_served_composition_completes_a_managed_decision_and_records_the_exact_bytes() {
    let composition = served_composition(true);
    let mut session = composition.open();

    let decision = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("a composition with a recording sink completes the managed decision");

    assert!(matches!(decision, Decision::Action { .. }));
    assert_eq!(composition.exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(
        composition
            .requests
            .lock()
            .expect("recorded requests")
            .as_slice(),
        std::slice::from_ref(&composition.expected)
    );

    let sink = composition
        .sink
        .as_ref()
        .expect("the composition attached a recording sink");
    let prepared = sink.state(TransportState::Prepared);
    assert_eq!(prepared.len(), 1, "one component reaches the composed sink");
    assert_eq!(
        prepared[0].boundary,
        CaptureBoundary::ExoSessionRequest.as_str()
    );
    let component = prepared[0]
        .component
        .as_ref()
        .expect("the composed sink records the component kind");
    assert_eq!(component.kind, CaptureComponentKind::Stdin);
    assert_eq!(component.ordinal, 0);
    assert_eq!(component.media_type, "application/json");
    assert_eq!(
        component.bytes, composition.expected,
        "the composed sink recorded the prepared bytes, not a second serialization"
    );
    assert_eq!(
        sink.state(TransportState::WriteCompleted).len(),
        1,
        "the composed sink records the completion"
    );
}

#[test]
fn the_served_composition_without_a_recording_sink_refuses_before_any_provider_write() {
    let composition = served_composition(false);
    let mut session = composition.open();

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a composition with no recording sink must refuse the managed decision");

    assert_eq!(error.code, "prepared_boundary_unsupported");
    assert_eq!(error.class, ErrorClass::Capability);
    assert_eq!(
        composition.exchanges.load(Ordering::SeqCst),
        0,
        "the refusal must precede every provider write"
    );
    assert!(
        composition
            .requests
            .lock()
            .expect("recorded requests")
            .is_empty()
    );
    assert!(composition.sink.is_none());
}

#[test]
fn the_served_composition_records_only_the_advertised_exo_boundary() {
    assert_eq!(
        adapter_support("exo").exact_boundary(),
        Some(CaptureBoundary::ExoSessionRequest)
    );
    assert_eq!(
        adapter_support("ollama").exact_boundary(),
        Some(CaptureBoundary::HttpBody)
    );
    assert!(
        ADVERTISED_EXACT_ADAPTERS
            .iter()
            .any(|(adapter_id, boundary)| *adapter_id == "ollama"
                && *boundary == CaptureBoundary::HttpBody),
        "the unrecorded Ollama boundary is an advertised, accounted residual"
    );

    let limits = selected_limits(SELECTED_ITEMS);
    let (source, config) = render_fixture();
    let (mut session, _, exchanges, _) =
        render_test_session(source, config, limits, Default::default(), None);
    session.boundary_capture = BoundaryCaptureSink::new(Box::new(ObservingSink::new()));

    session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the recording sink admits the served managed decision");
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);

    let receipt = session
        .boundary
        .receipt(&dispatch_id())
        .expect("the served boundary retains the receipt it recorded");
    // The recorded boundary is the Exo application boundary the composition wrote. No `http_body`
    // (Ollama) record exists, so the served path publishes no exactness for the unrecorded boundary.
    assert_eq!(receipt.adapter_id, "exo");
    assert_eq!(receipt.boundary, CaptureBoundary::ExoSessionRequest);
    assert_ne!(receipt.boundary, CaptureBoundary::HttpBody);
}
