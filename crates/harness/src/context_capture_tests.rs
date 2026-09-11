// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn metadata_mode_does_not_retain_content() {
    let mut capture = MemoryCapture::new(CaptureMode::Metadata, 4, 128).expect("config");
    capture
        .prepared(CaptureInput {
            execution_id: "model-execution-7",
            attempt_id: Some("attempt-1"),
            boundary: CaptureBoundary::HarnessRequest,
            bytes: b"synthetic content",
        })
        .expect("record");
    let record = capture.records().next().expect("record");
    assert!(record.content.is_none());
    assert!(record.sha256.is_none());
}

#[test]
fn queue_overflow_is_a_gap_and_states_do_not_fabricate_receipt() {
    let mut capture = MemoryCapture::new(CaptureMode::Memory, 1, 128).expect("config");
    for _ in 0..2 {
        capture
            .prepared(CaptureInput {
                execution_id: "model-execution-7",
                attempt_id: None,
                boundary: CaptureBoundary::ExoSessionRequest,
                bytes: b"input",
            })
            .expect("record");
    }
    assert_eq!(capture.dropped_entries(), 1);
    assert_ne!(
        TransportState::WriteCompleted,
        TransportState::ReceiptReported
    );
}

#[test]
fn repeated_input_with_distinct_attempts_keeps_distinct_snapshot_identity() {
    let mut capture = MemoryCapture::new(CaptureMode::Metadata, 8, 128).expect("config");
    for attempt_id in ["attempt-a", "attempt-b"] {
        capture
            .prepared(CaptureInput {
                execution_id: "model-execution-7",
                attempt_id: Some(attempt_id),
                boundary: CaptureBoundary::HttpBody,
                bytes: b"same input",
            })
            .expect("record");
    }
    let records = capture.records().collect::<Vec<_>>();
    assert_ne!(records[0].snapshot_id, records[1].snapshot_id);
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
}

#[test]
fn generated_attempt_ids_are_valid_and_unique() {
    let first = generated_capture_attempt_id("exo");
    let second = generated_capture_attempt_id("exo");
    assert_ne!(first, second);
    assert!(valid_identity(&first));
    assert!(valid_identity(&second));
}

#[test]
fn length_prefixed_identity_avoids_hyphenated_execution_attempt_collisions() {
    let mut capture = MemoryCapture::new(CaptureMode::Metadata, 8, 128).expect("config");
    for (execution_id, attempt_id) in [
        ("execution-a-b", "attempt-c"),
        ("execution-a", "attempt-b-c"),
    ] {
        capture
            .prepared(CaptureInput {
                execution_id,
                attempt_id: Some(attempt_id),
                boundary: CaptureBoundary::ExoSessionRequest,
                bytes: b"same input",
            })
            .expect("record");
    }
    let records = capture.records().collect::<Vec<_>>();
    assert_ne!(records[0].snapshot_id, records[1].snapshot_id);
}

#[test]
fn boundary_and_parent_linkage_are_preserved_for_lifecycle_records() {
    let mut capture = MemoryCapture::new(CaptureMode::Metadata, 8, 128).expect("config");
    capture
        .prepared(CaptureInput {
            execution_id: "model-execution-7",
            attempt_id: Some("attempt-a"),
            boundary: CaptureBoundary::ExoSessionRequest,
            bytes: b"input",
        })
        .expect("prepared");
    capture
        .write_completed_at(
            "model-execution-7",
            Some("attempt-a"),
            CaptureBoundary::ExoSessionRequest,
        )
        .expect("completed");
    let prepared_snapshot_id = capture
        .records()
        .next()
        .expect("prepared record")
        .snapshot_id
        .clone();
    assert_ne!(prepared_snapshot_id, {
        let mut other = MemoryCapture::new(CaptureMode::Metadata, 8, 128).expect("config");
        other
            .prepared(CaptureInput {
                execution_id: "model-execution-7",
                attempt_id: Some("attempt-a"),
                boundary: CaptureBoundary::HttpBody,
                bytes: b"input",
            })
            .expect("prepared");
        other
            .records()
            .next()
            .expect("other record")
            .snapshot_id
            .clone()
    });
    capture
        .write_unknown(
            "model-execution-7",
            Some("attempt-a"),
            "post_write_response_failure",
            CaptureBoundary::ExoSessionRequest,
        )
        .expect("unknown");
    let records = capture.records().collect::<Vec<_>>();
    assert_eq!(records[1].boundary, CaptureBoundary::ExoSessionRequest);
    assert_ne!(records[1].snapshot_id, prepared_snapshot_id);
    assert_eq!(
        records[1].parent_snapshot_id.as_deref(),
        Some(prepared_snapshot_id.as_str())
    );
    assert_eq!(
        capture.records().last().expect("unknown").state,
        TransportState::Unknown
    );
}

#[test]
fn private_memory_capture_is_rejected_until_an_approved_vault_is_supplied() {
    assert!(matches!(
        MemoryCapture::new(CaptureMode::Private, 4, 128),
        Err(CaptureError::PrivateRequiresVault)
    ));
}
