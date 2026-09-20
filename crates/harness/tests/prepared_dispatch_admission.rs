// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Admission and once-only conformance tests for prepared application input.
//!
//! These tests pin the refusals that precede a hold and the guarantees that follow a write:
//! unsupported adapters, malformed material, duplicate resume, a lost reply, disabled recording,
//! terminal cancellation and restart recovery.  Every fixture is synthetic and application-
//! controlled.  No test starts a provider, a host or a game process.

#[path = "prepared_dispatch/dispatch_fixtures.rs"]
mod fixtures;

use fixtures::{approved, base_fences, capture, components, digest, held};
use sts2_harness::context_capture::{
    ADVERTISED_EXACT_ADAPTERS, ApprovedDispatchMaterial, CaptureComponent, CaptureComponentKind,
    CaptureRecordingPort, DispatchError, DispatchOutcome, DispatchState, EffectiveContextClaim,
    NoopCapture, PreparedApplicationInput, PreparedDispatchController, PreparedDispatchPort,
    TransportState, adapter_support,
};

/// A write port whose transport outcome is indeterminate: it counts the attempt and reports a
/// lost reply without a recorded write.
#[derive(Debug, Default)]
struct LostReplyPort {
    attempts: u32,
}

impl PreparedDispatchPort for LostReplyPort {
    fn write_prepared(
        &mut self,
        _material: ApprovedDispatchMaterial<'_>,
    ) -> Result<usize, DispatchError> {
        self.attempts = self.attempts.saturating_add(1);
        Err(DispatchError::Indeterminate)
    }
}

#[test]
fn duplicate_resume_returns_the_retained_receipt_without_a_second_write() {
    let mut controller = PreparedDispatchController::new();
    let fences = held(&mut controller, "dispatch-once", "exo");
    let mut capture = capture();
    let (first, attempts) = {
        let mut port = CaptureRecordingPort::new(&mut capture);
        let receipt = controller
            .resume("dispatch-once", &fences, &mut port)
            .expect("resume");
        (receipt, port.write_attempts())
    };
    assert_eq!(attempts, 1);

    {
        let mut port = CaptureRecordingPort::new(&mut capture);
        assert_eq!(
            controller
                .resume("dispatch-once", &fences, &mut port)
                .expect("duplicate resume"),
            first
        );
        // Even a drifted fence cannot turn a recorded approval into a second write.
        let mut drifted = fences.clone();
        drifted.compaction_digest = digest("compaction-drifted");
        assert_eq!(
            controller
                .resume("dispatch-once", &drifted, &mut port)
                .expect("fenced duplicate resume"),
            first
        );
        assert_eq!(port.write_attempts(), 0);
    }
    assert_eq!(controller.boundary_writes(), 1);
    assert_eq!(controller.gameplay_effects(), 1);
    assert_eq!(controller.ledger().receipt_count(), 1);
    assert_eq!(
        capture
            .records()
            .filter(|record| record.state == TransportState::WriteCompleted)
            .count(),
        1
    );
}

#[test]
fn a_lost_reply_is_recorded_and_never_resent() {
    let mut controller = PreparedDispatchController::new();
    let fences = held(&mut controller, "dispatch-lost", "exo");
    let mut port = LostReplyPort::default();
    assert_eq!(
        controller
            .resume("dispatch-lost", &fences, &mut port)
            .unwrap_err(),
        DispatchError::Indeterminate
    );
    assert_eq!(port.attempts, 1);
    assert_eq!(controller.boundary_writes(), 1);
    assert_eq!(controller.gameplay_effects(), 0);

    let retained = controller
        .ledger()
        .receipt("dispatch-lost")
        .expect("receipt")
        .clone();
    assert_eq!(retained.outcome, DispatchOutcome::Unknown);
    assert_eq!(retained.manifest_sha256, approved("exo").manifest_sha256);
    assert_eq!(retained.gameplay_effects, 0);

    // Reconciliation reads the receipt; it never resends the approved material.
    assert_eq!(
        controller
            .resume("dispatch-lost", &fences, &mut port)
            .expect("reconciled resume"),
        retained
    );
    assert_eq!(port.attempts, 1);
    assert_eq!(controller.boundary_writes(), 1);
}

#[test]
fn cancellation_is_terminal_and_survives_a_restart() {
    let mut controller = PreparedDispatchController::new();
    let fences = base_fences("exo");
    controller
        .draft("dispatch-cancel", approved("exo"), fences.clone())
        .expect("draft");
    assert_eq!(
        controller.cancel("dispatch-cancel").expect("cancel"),
        DispatchState::Cancelled
    );
    assert_eq!(
        controller.state("dispatch-cancel"),
        Some(DispatchState::Cancelled)
    );

    let mut capture = capture();
    {
        let mut port = CaptureRecordingPort::new(&mut capture);
        assert_eq!(
            controller
                .resume("dispatch-cancel", &fences, &mut port)
                .unwrap_err(),
            DispatchError::Cancelled
        );
        assert_eq!(port.write_attempts(), 0);
    }
    assert_eq!(
        controller
            .draft("dispatch-cancel", approved("exo"), fences.clone())
            .unwrap_err(),
        DispatchError::Cancelled
    );

    let mut restarted = PreparedDispatchController::restart(controller.ledger().clone());
    assert!(restarted.ledger().is_cancelled("dispatch-cancel"));
    assert_eq!(
        restarted
            .draft("dispatch-cancel", approved("exo"), fences)
            .unwrap_err(),
        DispatchError::Cancelled
    );
    assert_eq!(restarted.boundary_writes(), 0);
    assert_eq!(capture.records().count(), 0);
}

#[test]
fn a_restart_cannot_dispatch_a_recorded_approval_again() {
    let mut controller = PreparedDispatchController::new();
    let fences = held(&mut controller, "dispatch-restart", "exo");
    let mut capture = capture();
    {
        let mut port = CaptureRecordingPort::new(&mut capture);
        let receipt = controller
            .resume("dispatch-restart", &fences, &mut port)
            .expect("resume");
        assert_eq!(receipt.outcome, DispatchOutcome::WriteCompleted);
        assert_eq!(port.write_attempts(), 1);
    }

    let mut restarted = PreparedDispatchController::restart(controller.ledger().clone());
    assert_eq!(restarted.ledger().receipt_count(), 1);
    assert_eq!(
        restarted
            .draft("dispatch-restart", approved("exo"), fences.clone())
            .unwrap_err(),
        DispatchError::DuplicateDispatch
    );
    {
        let mut port = CaptureRecordingPort::new(&mut capture);
        let retained = restarted
            .resume("dispatch-restart", &fences, &mut port)
            .expect("retained receipt");
        assert_eq!(retained.outcome, DispatchOutcome::WriteCompleted);
        assert_eq!(port.write_attempts(), 0);
    }
    assert_eq!(restarted.boundary_writes(), 0);
    assert_eq!(
        capture
            .records()
            .filter(|record| record.state == TransportState::WriteCompleted)
            .count(),
        1
    );
}

#[test]
fn an_unrecorded_write_cannot_produce_an_exact_claim() {
    let mut controller = PreparedDispatchController::new();
    let fences = held(&mut controller, "dispatch-unrecorded", "exo");
    let mut noop = NoopCapture;
    {
        let mut port = CaptureRecordingPort::new(&mut noop);
        assert_eq!(
            controller
                .resume("dispatch-unrecorded", &fences, &mut port)
                .unwrap_err(),
            DispatchError::CaptureDisabled
        );
        assert_eq!(port.write_attempts(), 1);
    }
    let retained = controller
        .ledger()
        .receipt("dispatch-unrecorded")
        .expect("receipt");
    assert_eq!(retained.outcome, DispatchOutcome::Unknown);
    assert_eq!(retained.gameplay_effects, 0);
    assert_eq!(controller.gameplay_effects(), 0);
}

#[test]
fn unsupported_adapters_never_claim_exact_effective_provider_context() {
    for adapter_id in ["studio-preview", "native-host", ""] {
        let support = adapter_support(adapter_id);
        assert_eq!(support.claim(), EffectiveContextClaim::Unsupported);
        assert_eq!(support.exact_boundary(), None);
        assert!(!support.claim().covers_provider_internal_context());
        assert_eq!(
            PreparedApplicationInput::prepare(
                adapter_id,
                "exec-fixture",
                None,
                &components("ollama")
            )
            .unwrap_err(),
            DispatchError::UnsupportedAdapter
        );
    }
    for (adapter_id, boundary) in ADVERTISED_EXACT_ADAPTERS {
        let support = adapter_support(adapter_id);
        assert_eq!(support.exact_boundary(), Some(boundary));
        assert_eq!(
            support.claim(),
            EffectiveContextClaim::ExactApplicationBoundary
        );
        // Even the exact claim stops at the application boundary.
        assert!(!support.claim().covers_provider_internal_context());
    }
}

#[test]
fn malformed_or_mismatched_material_is_refused_before_any_hold() {
    let oversized = vec![b'x'; 1_048_577];
    let oversized_component = [CaptureComponent {
        kind: CaptureComponentKind::Stdin,
        ordinal: 0,
        media_type: "text/plain",
        bytes: &oversized,
    }];
    assert_eq!(
        PreparedApplicationInput::prepare("exo", "exec-fixture", None, &oversized_component)
            .unwrap_err(),
        DispatchError::TooLarge
    );

    let unordered = [
        CaptureComponent {
            kind: CaptureComponentKind::Stdin,
            ordinal: 1,
            media_type: "text/plain",
            bytes: b"second",
        },
        CaptureComponent {
            kind: CaptureComponentKind::Stdin,
            ordinal: 1,
            media_type: "text/plain",
            bytes: b"first",
        },
    ];
    assert_eq!(
        PreparedApplicationInput::prepare("exo", "exec-fixture", None, &unordered).unwrap_err(),
        DispatchError::InvalidMaterial
    );

    let blank_media_type = [CaptureComponent {
        kind: CaptureComponentKind::Stdin,
        ordinal: 0,
        media_type: "",
        bytes: b"stdin-fixture",
    }];
    assert_eq!(
        PreparedApplicationInput::prepare("exo", "exec-fixture", None, &blank_media_type)
            .unwrap_err(),
        DispatchError::InvalidMaterial
    );

    assert_eq!(
        PreparedApplicationInput::prepare("exo", "exec-fixture", None, &[]).unwrap_err(),
        DispatchError::InvalidMaterial
    );
    let components = components("exo");
    assert_eq!(
        PreparedApplicationInput::prepare("exo", "exec-fixture", Some("bad id"), &components)
            .unwrap_err(),
        DispatchError::InvalidBinding
    );

    let mut controller = PreparedDispatchController::new();
    assert_eq!(
        controller
            .draft("dispatch-mismatch", approved("exo"), base_fences("ollama"))
            .unwrap_err(),
        DispatchError::UnsupportedAdapter
    );
    let mut broken = base_fences("exo");
    broken.compaction_digest = "not-a-digest".to_owned();
    assert_eq!(
        controller
            .draft("dispatch-broken", approved("exo"), broken)
            .unwrap_err(),
        DispatchError::InvalidBinding
    );
    assert_eq!(controller.boundary_writes(), 0);
    assert_eq!(controller.ledger().receipt_count(), 0);
}
