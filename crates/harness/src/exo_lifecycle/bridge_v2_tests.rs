// SPDX-License-Identifier: MIT

use super::{EXO_LIFECYCLE_WIRE_V2, parse_lifecycle_response};
use crate::ExoWireError;
use serde_json::json;

fn response() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "wire_version": EXO_LIFECYCLE_WIRE_V2,
        "request_id": "request-1",
        "turn_id": "host-turn-1",
        "outcome": "decision",
        "decision": {"decision": "wait", "rationale": "bounded"},
        "error_code": null,
        "native": {
            "agent_id": "019cb887-c7e8-7000-8000-000000000001",
            "conversation_id": "019cb887-c7e8-7000-8000-000000000002",
            "session_id": "019cb887-c7e8-7000-8000-000000000003",
            "turn_id": "019cb887-c7e8-7000-8000-000000000004",
            "event_cursor": "019cb887-c7e8-7000-8000-000000000005"
        }
    }))
}

#[test]
fn v2_response_keeps_native_and_host_turn_namespaces_distinct()
-> Result<(), Box<dyn std::error::Error>> {
    let (decision, native) = parse_lifecycle_response(&response()?, "request-1", "host-turn-1")?;
    assert!(matches!(decision, crate::Decision::Wait { .. }));
    assert_ne!(native.turn_id, "host-turn-1");
    Ok(())
}

#[test]
fn v2_refuses_swapped_host_correlation_and_missing_native_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value: serde_json::Value = serde_json::from_slice(&response()?)?;
    value["turn_id"] = json!("host-turn-2");
    let wire = serde_json::to_vec(&value)?;
    assert_eq!(
        parse_lifecycle_response(&wire, "request-1", "host-turn-1"),
        Err(ExoWireError::IdentityMismatch)
    );
    value["turn_id"] = json!("host-turn-1");
    value["native"] = serde_json::Value::Null;
    let wire = serde_json::to_vec(&value)?;
    assert_eq!(
        parse_lifecycle_response(&wire, "request-1", "host-turn-1"),
        Err(ExoWireError::InvalidIdentity)
    );
    Ok(())
}

#[test]
fn v2_refuses_duplicate_or_unknown_receipt_fields() -> Result<(), Box<dyn std::error::Error>> {
    let duplicate =
        br#"{"wire_version":"sts2.exo-bridge-wire-v2","wire_version":"sts2.exo-bridge-wire-v2"}"#;
    assert_eq!(
        parse_lifecycle_response(duplicate, "request-1", "host-turn-1"),
        Err(ExoWireError::DuplicateField)
    );
    let mut value: serde_json::Value = serde_json::from_slice(&response()?)?;
    value["native"]["untrusted"] = json!(true);
    let wire = serde_json::to_vec(&value)?;
    assert_eq!(
        parse_lifecycle_response(&wire, "request-1", "host-turn-1"),
        Err(ExoWireError::InvalidShape)
    );
    Ok(())
}
