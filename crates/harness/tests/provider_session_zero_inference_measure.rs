// SPDX-License-Identifier: MIT

//! #109 AC2 — the zero-inference property is *measured* on the production provider-continuity
//! broker path, not merely asserted by a type.
//!
//! A read/history operation must drive the measured provider-attempt counter to zero, while the
//! paired read counter proves the read actually ran. A genuine dispatch admission is the control:
//! it increments the provider-attempt counter. Unknown effective provider coverage stays
//! explicitly `unknown`.
//!
//! Synthetic fixtures only: an in-memory broker with the compiled-peer fixture capability profile.
//! No provider, native process, host, game or wall clock is contacted, so this is component
//! evidence for the continuity seam, not live-inference evidence.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::provider_session::*;

#[path = "support/provider_session_fixture.rs"]
mod fixture;

use fixture::{broker, held_binding, item, prepare};

/// AC2 (positive): a plain history view and a bounded history refresh both leave the measured
/// provider-attempt counter at zero, while the read counter proves the read path ran.
#[test]
fn read_and_history_operations_measure_zero_provider_attempts() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");

    // Unknown effective provider coverage must be reported as `unknown`, never promoted to
    // "covered".
    let view = broker
        .history(&binding.binding_id, None, 8)
        .expect("read history view");
    assert_eq!(view.coverage, HistoryCoverageView::Unknown);
    assert_eq!(view.effective_context_coverage, "unknown");
    assert!(!view.read_started_turn);

    // The pure view is not a refresh operation, and neither path may cause inference.
    assert_eq!(broker.history_read_count(), 0);
    assert_eq!(
        broker.provider_attempt_count(),
        0,
        "a history view must measure zero provider attempts"
    );

    broker
        .refresh_history(
            "owner-fixture",
            &binding.binding_id,
            "refresh-1",
            vec![item(1), item(2)],
            2,
            true,
        )
        .expect("refresh history");
    assert_eq!(
        broker.history_read_count(),
        1,
        "the refresh operation must actually have run"
    );
    assert_eq!(
        broker.provider_attempt_count(),
        0,
        "a read/history refresh must measure zero provider attempts"
    );

    // The refreshed coverage is still only the caller-supplied claim about application items; the
    // effective provider window stays unknown regardless.
    let view = broker
        .history(&binding.binding_id, None, 8)
        .expect("read history view");
    assert_eq!(view.effective_context_coverage, "unknown");
}

/// AC2 (control): a genuine dispatch admission increments the measured provider-attempt counter,
/// and an idempotent replay of that admission is not counted twice.
#[test]
fn dispatch_admission_measures_a_provider_attempt() {
    let mut broker = broker();
    let binding = held_binding(&mut broker);
    let prepared = prepare(&mut broker, &binding.binding_id, "prepared-dispatch");
    broker
        .explicit_resume("owner-fixture", &binding.binding_id)
        .expect("resume");
    assert_eq!(broker.provider_attempt_count(), 0);

    let operation = broker
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-1",
        )
        .expect("admit turn");
    assert_eq!(
        broker.provider_attempt_count(),
        1,
        "a dispatch admission must measure one provider attempt"
    );
    assert_eq!(
        broker.history_read_count(),
        0,
        "a dispatch admission is not a history read"
    );

    let replay = broker
        .admit_turn(
            "owner-fixture",
            &binding.binding_id,
            &prepared.prepared_id,
            "turn-1",
        )
        .expect("idempotent replay");
    assert_eq!(replay.operation_id, operation.operation_id);
    assert_eq!(
        broker.provider_attempt_count(),
        1,
        "an idempotent replay of one admission is not a second provider attempt"
    );
}

/// AC2 (durability guard): both counters are process-local measurements and are deliberately absent
/// from `BrokerSnapshot`. A broker that served a read, or admitted a turn, and is then restored from
/// its own snapshot starts both counters at zero rather than fabricating the activity it observed
/// before the restart.
#[test]
fn restored_broker_does_not_fabricate_attempt_counts() {
    // A served read/history refresh is measured, then snapshotted and restored.
    let mut read_broker = broker();
    let read_binding = held_binding(&mut read_broker);
    read_broker
        .explicit_resume("owner-fixture", &read_binding.binding_id)
        .expect("resume");
    read_broker
        .refresh_history(
            "owner-fixture",
            &read_binding.binding_id,
            "refresh-snapshot",
            vec![item(1)],
            1,
            true,
        )
        .expect("refresh history");
    assert_eq!(read_broker.history_read_count(), 1);

    let bytes = read_broker.snapshot_json().expect("snapshot");
    let restored_read =
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner").expect("restore");
    assert_eq!(
        restored_read.history_read_count(),
        0,
        "the read counter is process-local and must not be persisted in a snapshot"
    );
    assert_eq!(restored_read.provider_attempt_count(), 0);

    // A genuine dispatch admission is measured, then snapshotted and restored.
    let mut turn_broker = broker();
    let turn_binding = held_binding(&mut turn_broker);
    let prepared = prepare(
        &mut turn_broker,
        &turn_binding.binding_id,
        "prepared-snapshot",
    );
    turn_broker
        .explicit_resume("owner-fixture", &turn_binding.binding_id)
        .expect("resume");
    turn_broker
        .admit_turn(
            "owner-fixture",
            &turn_binding.binding_id,
            &prepared.prepared_id,
            "turn-snapshot",
        )
        .expect("admit turn");
    assert_eq!(turn_broker.provider_attempt_count(), 1);

    let bytes = turn_broker.snapshot_json().expect("snapshot");
    let restored_turn =
        ProviderSessionBroker::from_snapshot_json(&bytes, "replacement-owner").expect("restore");
    assert_eq!(
        restored_turn.provider_attempt_count(),
        0,
        "the provider-attempt counter is process-local and must not be persisted in a snapshot"
    );
    assert_eq!(restored_turn.history_read_count(), 0);
}
