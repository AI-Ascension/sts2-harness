// SPDX-License-Identifier: MIT

use super::values::{positive_u53, strict_timestamp};
use super::*;
use serde_json::json;

fn valid_frame(kind: &str) -> Value {
    let payload = if kind == "operation_lookup_response" {
        json!({
            "result": {"status":"SETTLED","retryable":false,"retry_after_seconds":null},
            "operation": null,
            "mutation_authorized": false
        })
    } else {
        json!({
            "result": {"status":"RECONCILED","retryable":false,"retry_after_seconds":null},
            "operation": null,
            "witness": null
        })
    };
    json!({
        "contract": "watchdog-recovery-v1",
        "schema_digest": sts2_harness::RECOVERY_SCHEMA_DIGEST,
        "message_id": "550c0000-5555-4555-8555-555555555555",
        "correlation_id": "66666666-6666-4666-8666-666666666666",
        "sent_at": "2026-09-06T22:30:00Z",
        "actor": {"principal_id":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","role":"gateway"},
        "auth": {"principal_id":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","capability": if kind == "operation_lookup_response" {"recovery_read"} else {"recovery_reconcile"},"proof":null},
        "kind": kind,
        "payload": payload
    })
}

#[test]
fn raw_duplicate_and_unknown_fields_are_rejected_before_value_use() {
    let frame = valid_frame("operation_lookup_response").to_string();
    assert!(decode_frame(&frame, Some("operation_lookup_response"), None).is_ok());
    let duplicate = frame.replacen(
        "\"contract\":\"watchdog-recovery-v1\"",
        "\"contract\":\"watchdog-recovery-v1\",\"contract\":\"watchdog-recovery-v1\"",
        1,
    );
    assert!(decode_frame(&duplicate, Some("operation_lookup_response"), None).is_err());
    let unknown = frame.replacen(
        "\"kind\":\"operation_lookup_response\"",
        "\"kind\":\"operation_lookup_response\",\"private\":true",
        1,
    );
    assert!(decode_frame(&unknown, Some("operation_lookup_response"), None).is_err());
}

#[test]
fn correlation_and_kind_must_echo_expected_values() {
    let frame = valid_frame("operation_lookup_response").to_string();
    assert!(
        decode_frame(
            &frame,
            Some("operation_lookup_response"),
            Some("66666666-6666-4666-8666-666666666666")
        )
        .is_ok()
    );
    assert!(
        decode_frame(
            &frame,
            Some("operation_reconcile_response"),
            Some("66666666-6666-4666-8666-666666666666")
        )
        .is_err()
    );
    assert!(
        decode_frame(
            &frame,
            Some("operation_lookup_response"),
            Some("77777777-7777-4777-8777-777777777777")
        )
        .is_err()
    );
}

#[test]
fn strict_utc_timestamps_and_positive_wire_numbers_are_bounded() {
    assert!(strict_timestamp("2026-02-28T23:59:59.123456789Z"));
    assert!(!strict_timestamp("2026-02-29T23:59:59Z"));
    assert!(!strict_timestamp("2026-02-28T23:59:59+00:00"));
    assert!(positive_u53(&json!(1)).is_some());
    assert!(positive_u53(&json!(0)).is_none());
    assert!(positive_u53(&json!(1.0)).is_none());
    assert!(positive_u53(&json!(9_007_199_254_740_992_u64)).is_none());
}
