// SPDX-License-Identifier: MIT

//! Served acceptance for the recorded prepared-application boundary.
//!
//! Every test here drives the real `ProductionLiveWorkflowSession::decide_for` path. The observed
//! recording is the only source of the approved-versus-written evidence they assert, so removing
//! the recording write port from the served decision makes them fail rather than merely reduce
//! coverage.

use super::managed_render_tests::{render_fixture, render_test_session, selected_limits};
use super::*;
use crate::context_capture::{
    ADVERTISED_EXACT_ADAPTERS, CaptureBoundary, CaptureComponent, CaptureComponentKind,
    CaptureError, CaptureInput, CapturePort, DispatchError, DispatchOutcome, EffectiveContextClaim,
    PreparedApplicationInput, TransportState, adapter_support,
};
use crate::management::{ContextRenderSource, ErrorClass};
use crate::{ContextRenderLimits, ExoConfig};
use std::sync::atomic::AtomicBool;

/// Selected limits that admit the fixture's two retained items.
pub(super) const SELECTED_ITEMS: usize = 2;

/// The dispatch identity the served boundary derives for one model execution.
///
/// Pinned here instead of imported, so a change to the served identity scheme fails this test
/// rather than silently moving the identity an operator reconciles against.
fn dispatch_id(execution_id: &str) -> String {
    format!("exo.{}", &crate::sha256_hex(execution_id.as_bytes())[..32])
}

/// One recorded observation of the served boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
struct SinkRecord {
    state: TransportState,
    boundary: String,
    component: Option<ObservedComponent>,
}

/// One exact component the sink observed, including the bytes themselves.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedComponent {
    kind: CaptureComponentKind,
    ordinal: u16,
    media_type: String,
    bytes: Vec<u8>,
}

fn lifecycle(state: TransportState, boundary: CaptureBoundary) -> SinkRecord {
    SinkRecord {
        state,
        boundary: boundary.as_str().to_owned(),
        component: None,
    }
}

/// A recording sink that keeps what the served boundary actually handed it.
#[derive(Clone, Debug)]
pub(super) struct ObservingSink {
    records: Arc<Mutex<Vec<SinkRecord>>>,
}

impl ObservingSink {
    pub(super) fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn state(&self, state: TransportState) -> Vec<SinkRecord> {
        self.records
            .lock()
            .expect("sink records")
            .iter()
            .filter(|record| record.state == state)
            .cloned()
            .collect()
    }

    fn push(&self, record: SinkRecord) {
        self.records.lock().expect("sink records").push(record);
    }

    /// The sink's recorded (prepared, completed) depth so far.
    ///
    /// Sampled from inside the provider exchange, this witnesses the record/exchange interleaving.
    pub(super) fn depth(&self) -> (usize, usize) {
        (
            self.state(TransportState::Prepared).len(),
            self.state(TransportState::WriteCompleted).len(),
        )
    }
}

impl CapturePort for ObservingSink {
    fn enabled(&self) -> bool {
        true
    }

    /// A legacy single-byte observation is recorded without a component kind, so a regression away
    /// from the structured component path is visible instead of being silently accepted.
    fn prepared(&mut self, input: CaptureInput<'_>) -> Result<(), CaptureError> {
        self.push(SinkRecord {
            state: TransportState::Prepared,
            boundary: input.boundary.as_str().to_owned(),
            component: None,
        });
        Ok(())
    }

    fn prepared_component(
        &mut self,
        input: CaptureInput<'_>,
        kind: CaptureComponentKind,
        ordinal: u16,
        media_type: &str,
    ) -> Result<(), CaptureError> {
        self.push(SinkRecord {
            state: TransportState::Prepared,
            boundary: input.boundary.as_str().to_owned(),
            component: Some(ObservedComponent {
                kind,
                ordinal,
                media_type: media_type.to_owned(),
                bytes: input.bytes.to_vec(),
            }),
        });
        Ok(())
    }

    fn write_completed(&mut self, _execution_id: &str) -> Result<(), CaptureError> {
        self.push(lifecycle(
            TransportState::WriteCompleted,
            CaptureBoundary::HttpBody,
        ));
        Ok(())
    }

    fn write_completed_at(
        &mut self,
        _execution_id: &str,
        _attempt_id: Option<&str>,
        boundary: CaptureBoundary,
    ) -> Result<(), CaptureError> {
        self.push(lifecycle(TransportState::WriteCompleted, boundary));
        Ok(())
    }

    fn write_failed(&mut self, _execution_id: &str, _code: &str) -> Result<(), CaptureError> {
        self.push(lifecycle(
            TransportState::WriteFailed,
            CaptureBoundary::HttpBody,
        ));
        Ok(())
    }

    fn write_unknown(
        &mut self,
        _execution_id: &str,
        _attempt_id: Option<&str>,
        _code: &str,
        boundary: CaptureBoundary,
    ) -> Result<(), CaptureError> {
        self.push(lifecycle(TransportState::Unknown, boundary));
        Ok(())
    }
}

/// The exact bytes the selected render source determines for `input()`.
///
/// Recomputed independently of the served path so the recording is compared with the approved
/// material rather than with itself.
fn expected_provider_bytes(
    source: &ContextRenderSource,
    config: &ExoConfig,
    limits: ContextRenderLimits,
) -> Vec<u8> {
    let input = input();
    let managed = crate::context_control::ManagedRenderInput {
        execution_id: input.execution_id.to_string(),
        state_id: input.observation.state_id().to_owned(),
        generation: input.observation.generation(),
        observation: input.observation.fair_play().as_value().clone(),
        legal_action_ids: input
            .legal_actions
            .actions()
            .iter()
            .map(|action| action.action_id().to_owned())
            .collect(),
        objective: input.objective.clone(),
        hard_constraints: input.hard_constraints.clone(),
        map_context: None,
    };
    crate::context_control::ContextRenderer::enabled_at_with_limits(
        &source.boundary,
        managed,
        &source.document.draft,
        &source.document.items,
        config,
        source.now,
        &limits,
    )
    .expect("the selected source prepares the expected bytes")
    .provider_bytes()
    .to_vec()
}

/// A served session, the sink attached to its boundary, and the bytes that decision must write.
struct BoundaryFixture {
    session: ProductionLiveWorkflowSession,
    sink: ObservingSink,
    expected: Vec<u8>,
    exchanges: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

fn boundary_fixture(limits: ContextRenderLimits, stale: bool) -> BoundaryFixture {
    let (source, config) = render_fixture();
    let expected = expected_provider_bytes(&source, &config, limits);
    let (mut session, _, exchanges, requests) = render_test_session(
        source,
        config,
        limits,
        Arc::new(AtomicBool::new(stale)),
        None,
    );
    let sink = ObservingSink::new();
    session.boundary_capture = BoundaryCaptureSink::new(Box::new(sink.clone()));
    BoundaryFixture {
        session,
        sink,
        expected,
        exchanges,
        requests,
    }
}

#[test]
fn served_managed_boundary_records_the_exact_approved_bytes_before_the_write() {
    let BoundaryFixture {
        mut session,
        sink,
        expected,
        exchanges,
        requests,
    } = boundary_fixture(selected_limits(SELECTED_ITEMS), false);

    let decision = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the recorded managed decision is admitted");

    assert!(matches!(decision, Decision::Action { .. }));
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(
        requests.lock().expect("recorded requests").as_slice(),
        std::slice::from_ref(&expected)
    );

    let prepared = sink.state(TransportState::Prepared);
    assert_eq!(prepared.len(), 1, "one component reaches the sink");
    assert_eq!(
        prepared[0].boundary,
        CaptureBoundary::ExoSessionRequest.as_str()
    );
    let component = prepared[0]
        .component
        .as_ref()
        .expect("the served boundary records its component kind");
    assert_eq!(component.kind, CaptureComponentKind::Stdin);
    assert_eq!(component.ordinal, 0);
    assert_eq!(component.media_type, "application/json");
    assert_eq!(component.bytes, expected);

    let completed = sink.state(TransportState::WriteCompleted);
    assert_eq!(completed.len(), 1, "one completion is recorded");
    assert_eq!(
        completed[0].boundary,
        CaptureBoundary::ExoSessionRequest.as_str()
    );

    let receipt = session
        .boundary
        .receipt(&dispatch_id("model-execution-1"))
        .expect("the served boundary retains the receipt it recorded");
    assert_eq!(receipt.execution_id, "model-execution-1");
    assert_eq!(receipt.adapter_id, "exo");
    assert_eq!(receipt.boundary, CaptureBoundary::ExoSessionRequest);
    assert_eq!(receipt.outcome, DispatchOutcome::WriteCompleted);
    assert_eq!(receipt.boundary_writes, 1);
    assert_eq!(receipt.gameplay_effects, 1);
    assert_eq!(receipt.written_bytes, expected.len());
    // The manifest is recomputed over the observed component, so a mismatch in any bound field --
    // adapter, execution, boundary, kind, ordinal, media type or the bytes themselves -- shows here.
    let approved = PreparedApplicationInput::prepare(
        "exo",
        &receipt.execution_id,
        receipt.attempt_id.as_deref(),
        &[CaptureComponent {
            kind: component.kind,
            ordinal: component.ordinal,
            media_type: &component.media_type,
            bytes: &component.bytes,
        }],
    )
    .expect("the observed component is prepared material");
    assert_eq!(
        receipt.approved_material_sha256,
        approved.approved_material_sha256
    );
    assert_eq!(receipt.manifest_sha256, approved.manifest_sha256);
}

#[test]
fn served_managed_boundary_refuses_without_a_recording_sink() {
    let (source, config) = render_fixture();
    let limits = selected_limits(SELECTED_ITEMS);
    let (mut session, _, exchanges, _) =
        render_test_session(source, config, limits, Default::default(), None);
    // This is the sink the served factory ships with: a composition that enables managed rendering
    // without attaching an operator-built sink must refuse rather than publish exactness.
    session.boundary_capture = BoundaryCaptureSink::disabled();

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("the disabled default sink cannot record an exact boundary");

    assert_eq!(error.code, "prepared_boundary_unsupported");
    assert_eq!(error.class, ErrorClass::Capability);
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
    assert!(
        session
            .boundary
            .receipt(&dispatch_id("model-execution-1"))
            .is_none()
    );
}

#[test]
fn served_managed_boundary_fences_a_stale_source_before_the_write() {
    let limits = selected_limits(SELECTED_ITEMS);
    let BoundaryFixture {
        mut session,
        sink,
        exchanges,
        ..
    } = boundary_fixture(limits, true);

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("a source that drifted is fenced before the boundary is written");

    assert_eq!(error.code, "context_render_source_stale");
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);
    assert!(sink.state(TransportState::Prepared).is_empty());
    assert!(sink.state(TransportState::WriteCompleted).is_empty());
    assert!(
        session
            .boundary
            .receipt(&dispatch_id("model-execution-1"))
            .is_none()
    );
}

#[test]
fn served_managed_boundary_never_writes_one_approval_twice() {
    let limits = selected_limits(SELECTED_ITEMS);
    let BoundaryFixture {
        mut session,
        sink,
        exchanges,
        ..
    } = boundary_fixture(limits, false);

    session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the first recorded managed decision is admitted");
    let retained = session
        .boundary
        .receipt(&dispatch_id("model-execution-1"))
        .expect("the first write retains its receipt")
        .clone();

    let error = session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect_err("the retained receipt forbids a second write");

    assert_eq!(error.code, "prepared_boundary_already_recorded");
    assert_eq!(error.class, ErrorClass::Unresolved);
    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(sink.state(TransportState::Prepared).len(), 1);
    assert_eq!(sink.state(TransportState::WriteCompleted).len(), 1);
    assert_eq!(
        session
            .boundary
            .receipt(&dispatch_id("model-execution-1"))
            .expect("the receipt is retained unchanged"),
        &retained
    );
}

#[test]
fn served_boundary_never_claims_exactness_for_an_unadvertised_adapter() {
    let bytes = b"{\"decision\":\"wait\"}";
    let components = [CaptureComponent {
        kind: CaptureComponentKind::Stdin,
        ordinal: 0,
        media_type: "application/json",
        bytes,
    }];

    assert_eq!(
        adapter_support("native").claim(),
        EffectiveContextClaim::Unsupported
    );
    assert!(adapter_support("native").exact_boundary().is_none());
    assert!(
        !ADVERTISED_EXACT_ADAPTERS
            .iter()
            .any(|(adapter_id, _)| *adapter_id == "native")
    );
    assert_eq!(
        PreparedApplicationInput::prepare("native", "model-execution-1", None, &components),
        Err(DispatchError::UnsupportedAdapter)
    );
    assert_eq!(
        adapter_support("exo").exact_boundary(),
        Some(CaptureBoundary::ExoSessionRequest)
    );
}

#[test]
fn the_served_recording_default_can_record_a_boundary() {
    // The served composition attaches this sink. It must be able to record, or every served managed
    // decision would refuse with `prepared_boundary_unsupported` before reaching the provider.
    let sink =
        BoundaryCaptureSink::memory_ring().expect("the served ring bounds are the module limits");
    assert!(
        sink.lock().expect("the served sink locks").enabled(),
        "the served default must be a recording sink, not the inert default"
    );
}
