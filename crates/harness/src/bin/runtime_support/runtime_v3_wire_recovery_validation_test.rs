// SPDX-License-Identifier: MIT

use super::values::{positive_u53, strict_timestamp};
use super::*;
use crate::runtime_support::runtime_v3::combat_demo;
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

#[test]
fn raw_error_messages_do_not_reflect_untrusted_fields_or_values() {
    let private = "untrusted-secret-marker";
    for raw in [
        format!(r#"{{"{private}":1,"{private}":2}}"#),
        format!(r#"{{"proof":{private}}}"#),
        format!("{} {private}", valid_frame("operation_lookup_response")),
    ] {
        let error = decode_frame(&raw, None, None);
        assert!(error.is_err());
        assert!(!error.err().unwrap_or_default().contains(private));
    }
    assert_eq!(
        decode_frame(&" ".repeat(262_145), None, None),
        Err(String::from("recovery frame exceeds the byte limit"))
    );
}

#[test]
fn recovery_response_actor_must_bind_the_gateway_auth_principal() {
    let mut frame = valid_frame("operation_lookup_response");
    frame["actor"]["role"] = json!("harness");
    assert!(decode_frame(&frame.to_string(), None, None).is_err());
    frame["actor"]["role"] = json!("gateway");
    frame["auth"]["principal_id"] = json!("77777777-7777-4777-8777-777777777777");
    assert!(decode_frame(&frame.to_string(), None, None).is_err());
}

#[test]
fn production_combat_operation_id_is_accepted_at_the_recovery_boundary() {
    let operation_id = combat_demo::new_operation_id();
    let mut frame = valid_frame("operation_lookup_response");
    frame["payload"]["operation"] = json!({
        "operation_id": operation_id,
        "state": "SETTLED",
        "payload_digest": "150dbfb8ac3331371ecd80224e78274a8aa1d221916d867b6125bfe07ac88107",
        "original_context": {
            "deployment_id": "33333333-3333-4333-8333-333333333333",
            "instance_id": "44444444-4444-4444-8444-444444444444",
            "instance_incarnation": "55555555-5555-4555-8555-555555555555",
            "boot_id": "66666666-6666-4666-8666-666666666666",
            "authority_generation": 1,
            "lease_id": "77777777-7777-4777-8777-777777777777",
            "lease_epoch": 1
        },
        "expected_boundary": {
            "state_id": "22222222-2222-4222-8222-222222222222",
            "generation": 0,
            "catalog_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        },
        "action": {
            "schema_digest": "8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63",
            "canonical_json_b64": "eyJhY3Rpb24iOnsia2luZCI6ImVuZF90dXJuIn0sImFjdGlvbl9pZCI6ImNvbWJhdC5lbmQtdHVybiJ9",
            "payload_digest": "150dbfb8ac3331371ecd80224e78274a8aa1d221916d867b6125bfe07ac88107"
        },
        "ticket": null,
        "witness": null,
        "uncertainty_reason": null,
        "created_at": "2026-09-07T00:00:00Z",
        "updated_at": "2026-09-07T00:00:01Z"
    });
    assert!(decode_frame(&frame.to_string(), Some("operation_lookup_response"), None).is_ok());
}
