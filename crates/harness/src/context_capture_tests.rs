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
                boundary: CaptureBoundary::ProviderRequest,
                bytes: b"same input",
            })
            .expect("record");
    }
    let records = capture.records().collect::<Vec<_>>();
    assert_ne!(records[0].snapshot_id, records[1].snapshot_id);
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
}
