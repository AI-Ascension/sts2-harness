// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Exactness conformance tests for prepared application input held at provider boundaries.
//!
//! These tests pin the approved manifest, the byte-free preview, the bound drift axes and stop
//! dominance.  Every fixture is synthetic and application-controlled.  No test starts a provider, a
//! host or a game process, and no test claims provider-internal context.

#[path = "prepared_dispatch/dispatch_fixtures.rs"]
mod fixtures;

use fixtures::{approved, base_fences, capture, digest, held};
use sts2_harness::context_capture::{
    ADVERTISED_EXACT_ADAPTERS, BoundaryManifestEntry, CaptureRecordingPort, DispatchError,
    DispatchFences, DispatchOutcome, DispatchState, DriftAxis, EffectiveContextClaim,
    PreparedDispatchController, TransportState, adapter_support, manifest_sha256, material_sha256,
};
use sts2_harness::hex_bytes;

#[test]
fn every_advertised_exact_adapter_records_byte_identical_approved_material() {
    for (adapter_id, boundary) in ADVERTISED_EXACT_ADAPTERS {
        assert_eq!(adapter_support(adapter_id).exact_boundary(), Some(boundary));
        let input = approved(adapter_id);
        assert_eq!(input.boundary, boundary);
        assert_eq!(
            input.claim(),
            EffectiveContextClaim::ExactApplicationBoundary
        );
        input
            .verify()
            .expect("approved digests match retained bytes");

        let mut controller = PreparedDispatchController::new();
        let fences = held(&mut controller, "dispatch-exact", adapter_id);

        let mut capture = capture();
        let (receipt, attempts) = {
            let mut port = CaptureRecordingPort::new(&mut capture);
            let receipt = controller
                .resume("dispatch-exact", &fences, &mut port)
                .expect("resume");
            (receipt, port.write_attempts())
        };
        let records: Vec<_> = capture.records().cloned().collect();

        // The recording write port observed exactly one write of exactly the approved material.
        assert_eq!(attempts, 1);
        assert_eq!(receipt.outcome, DispatchOutcome::WriteCompleted);
        assert_eq!(receipt.boundary, boundary);
        assert_eq!(receipt.written_bytes, input.approved_bytes());
        assert_eq!(receipt.manifest_sha256, input.manifest_sha256);
        assert_eq!(
            receipt.approved_material_sha256,
            input.approved_material_sha256
        );

        let prepared: Vec<_> = records
            .iter()
            .filter(|record| record.state == TransportState::Prepared)
            .collect();
        assert_eq!(prepared.len(), input.component_count());
        let mut observed_entries = Vec::new();
        let mut observed_chunks: Vec<Vec<u8>> = Vec::new();
        for (record, component) in prepared.iter().zip(input.components()) {
            assert_eq!(record.content.as_deref(), Some(component.bytes()));
            assert_eq!(record.observed_bytes, component.observed_bytes);
            assert_eq!(record.component_kind, Some(component.kind));
            assert_eq!(record.ordinal, Some(component.ordinal));
            assert_eq!(
                record.media_type.as_deref(),
                Some(component.media_type.as_str())
            );
            assert_eq!(record.boundary, boundary);
            assert_eq!(record.execution_id, input.execution_id);
            assert_eq!(record.attempt_id, input.attempt_id);
            assert_eq!(record.parent_snapshot_id, None);
            observed_entries.push(BoundaryManifestEntry {
                kind: component.kind,
                ordinal: component.ordinal,
                media_type: component.media_type.clone(),
                observed_bytes: record.observed_bytes,
                sha256: hex_bytes(record.sha256.expect("digest")),
            });
            observed_chunks.push(record.content.clone().expect("content"));
        }

        // The observed manifest and material digests equal the approved ones.
        assert_eq!(
            manifest_sha256(
                adapter_id,
                &input.execution_id,
                input.attempt_id.as_deref(),
                boundary,
                &observed_entries,
            ),
            input.manifest_sha256
        );
        let chunks: Vec<&[u8]> = observed_chunks.iter().map(Vec::as_slice).collect();
        assert_eq!(material_sha256(&chunks), input.approved_material_sha256);

        let completed: Vec<_> = records
            .iter()
            .filter(|record| record.state == TransportState::WriteCompleted)
            .collect();
        assert_eq!(completed.len(), 1);
        assert_eq!(
            completed[0].parent_snapshot_id.as_deref(),
            Some(prepared[0].snapshot_id.as_str())
        );
    }
}

#[test]
fn draft_preview_commit_and_metadata_reads_cause_no_write_or_inference() {
    let mut controller = PreparedDispatchController::new();
    let fences = base_fences("exo");
    controller
        .draft("dispatch-held", approved("exo"), fences.clone())
        .expect("draft");

    let preview = controller.preview("dispatch-held").expect("preview");
    assert_eq!(preview.state, DispatchState::Drafted);
    assert_eq!(preview.entries.len(), 3);
    assert!(!preview.bytes_exposed);

    assert_eq!(
        controller.commit("dispatch-held", &fences).expect("commit"),
        DispatchState::CommittedHeld
    );

    let metadata = controller.metadata("dispatch-held").expect("metadata");
    assert_eq!(metadata.state, DispatchState::CommittedHeld);
    assert_eq!(metadata.component_count, 3);
    assert_eq!(
        metadata.claim,
        EffectiveContextClaim::ExactApplicationBoundary
    );
    assert_eq!(metadata.receipt, None);

    // Repeated bounded reads still change nothing.
    assert!(controller.preview("dispatch-held").is_ok());
    assert!(controller.metadata("dispatch-held").is_ok());

    assert_eq!(
        controller.state("dispatch-held"),
        Some(DispatchState::CommittedHeld)
    );
    assert_eq!(controller.boundary_writes(), 0);
    assert_eq!(controller.gameplay_effects(), 0);
    assert_eq!(controller.ledger().receipt_count(), 0);

    let mut capture = capture();
    {
        let port = CaptureRecordingPort::new(&mut capture);
        assert_eq!(port.write_attempts(), 0);
    }
    assert_eq!(capture.records().count(), 0);
}

type AxisMutation = fn(&mut DispatchFences);

#[test]
fn drift_on_each_bound_axis_fences_the_approval_before_write() {
    let cases: [(DriftAxis, AxisMutation); 14] = [
        (DriftAxis::Adapter, |fences: &mut DispatchFences| {
            fences.adapter_id = "ollama".to_owned();
        }),
        (DriftAxis::Model, |fences: &mut DispatchFences| {
            fences.model_id = "model-drifted".to_owned();
        }),
        (DriftAxis::Configuration, |fences: &mut DispatchFences| {
            fences.configuration_digest = digest("configuration-drifted");
        }),
        (DriftAxis::State, |fences: &mut DispatchFences| {
            fences.state_digest = digest("state-drifted");
        }),
        (DriftAxis::Catalog, |fences: &mut DispatchFences| {
            fences.catalog_digest = digest("catalog-drifted");
        }),
        (DriftAxis::Profile, |fences: &mut DispatchFences| {
            fences.profile_digest = digest("profile-drifted");
        }),
        (DriftAxis::Auth, |fences: &mut DispatchFences| {
            fences.auth_digest = digest("auth-drifted");
        }),
        (DriftAxis::History, |fences: &mut DispatchFences| {
            fences.history_digest = digest("history-drifted");
        }),
        (DriftAxis::Compaction, |fences: &mut DispatchFences| {
            fences.compaction_digest = digest("compaction-drifted");
        }),
        (DriftAxis::Policy, |fences: &mut DispatchFences| {
            fences.policy_version += 1;
        }),
        (DriftAxis::Controller, |fences: &mut DispatchFences| {
            fences.controller_epoch += 1;
        }),
        (DriftAxis::Gate, |fences: &mut DispatchFences| {
            fences.gate_epoch += 1;
        }),
        (DriftAxis::Lease, |fences: &mut DispatchFences| {
            fences.lease_epoch += 1;
        }),
        (DriftAxis::Revocation, |fences: &mut DispatchFences| {
            fences.revocation_epoch += 1;
        }),
    ];

    for (axis, mutate) in cases {
        // Drift discovered while the approval is still a draft.
        let mut controller = PreparedDispatchController::new();
        let fences = base_fences("exo");
        controller
            .draft("dispatch-drift", approved("exo"), fences.clone())
            .expect("draft");
        let mut drifted = fences.clone();
        mutate(&mut drifted);
        assert_eq!(
            controller.commit("dispatch-drift", &drifted).unwrap_err(),
            DispatchError::Drift(axis)
        );
        assert_eq!(
            controller.state("dispatch-drift"),
            Some(DispatchState::Stale)
        );
        assert_eq!(controller.boundary_writes(), 0);

        // Drift discovered after the hold, immediately before the write.
        let mut controller = PreparedDispatchController::new();
        let fences = held(&mut controller, "dispatch-drift", "exo");
        let mut drifted = fences.clone();
        mutate(&mut drifted);
        let mut capture = capture();
        {
            let mut port = CaptureRecordingPort::new(&mut capture);
            assert_eq!(
                controller
                    .resume("dispatch-drift", &drifted, &mut port)
                    .unwrap_err(),
                DispatchError::Drift(axis)
            );
            assert_eq!(port.write_attempts(), 0);
        }
        assert_eq!(
            controller.state("dispatch-drift"),
            Some(DispatchState::Stale)
        );
        assert_eq!(controller.boundary_writes(), 0);
        assert_eq!(controller.gameplay_effects(), 0);
        assert_eq!(controller.ledger().receipt_count(), 0);
        assert_eq!(capture.records().count(), 0);
    }
}

#[test]
fn a_stop_request_dominates_a_held_approval() {
    let mut controller = PreparedDispatchController::new();
    let fences = held(&mut controller, "dispatch-stop", "exo");
    assert_eq!(controller.request_stop(), 1);
    assert_eq!(controller.request_stop(), 2);
    assert_eq!(
        controller.state("dispatch-stop"),
        Some(DispatchState::Stale)
    );

    let mut capture = capture();
    {
        let mut port = CaptureRecordingPort::new(&mut capture);
        assert_eq!(
            controller
                .resume("dispatch-stop", &fences, &mut port)
                .unwrap_err(),
            DispatchError::Stale
        );
        assert_eq!(port.write_attempts(), 0);
    }
    assert_eq!(controller.boundary_writes(), 0);
    assert_eq!(controller.gameplay_effects(), 0);
    assert_eq!(capture.records().count(), 0);
}
