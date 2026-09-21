// SPDX-License-Identifier: MIT

//! Ordering acceptance for the served prepared-application boundary.
//!
//! The served port records the approved material first and performs the provider exchange second.
//! That offset is invisible to the recording sink alone -- either order leaves exactly one component
//! record and one completion -- so the interleaving is witnessed from inside the provider exchange,
//! where the sink's preparation depth at that instant separates the two orders. Removing the
//! recording write port, or moving it after the exchange, fails this test rather than shrinking it.

use super::boundary_tests::{ObservingSink, SELECTED_ITEMS};
use super::managed_render_tests::{render_fixture, render_test_session, selected_limits};
use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn served_managed_boundary_records_the_approval_before_the_provider_exchange() {
    let limits = selected_limits(SELECTED_ITEMS);
    let (source, config) = render_fixture();
    let sink = ObservingSink::new();
    // Sampled while the exchange is running, so one order reads a prepared component and the other
    // reads none: the provider is called only after the approval it is about to write was recorded.
    let prober = sink.clone();
    let observed: Arc<Mutex<Option<(usize, usize)>>> = Arc::new(Mutex::new(None));
    let recorder = Arc::clone(&observed);
    let on_exchange: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        *recorder.lock().expect("exchange observation") = Some(prober.depth());
    });
    let (mut session, _, exchanges, _) = render_test_session(
        source,
        config,
        limits,
        Arc::new(AtomicBool::new(false)),
        Some(on_exchange),
    );
    session.boundary_capture = BoundaryCaptureSink::new(Box::new(sink.clone()));

    session
        .decide_for(&input(), "decision.live.v1", "context.live.v1")
        .expect("the recorded managed decision is admitted");

    assert_eq!(exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(
        *observed.lock().expect("exchange observation"),
        Some((1, 0)),
        "the approval is recorded before the exchange, and the completion only after it"
    );
}
