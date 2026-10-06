// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};
fn result() -> Result<SendResult, serde_json::Error> {
    Ok(SendResult {
        session_id: serde_json::from_value(json!("12345678-1234-4234-8234-123456789abc"))?,
        turn_id: serde_json::from_value(json!("22345678-1234-4234-8234-123456789abc"))?,
        latest_event_id: serde_json::from_value(json!("52345678-1234-4234-8234-123456789abc"))?,
    })
}
fn event(data: Value) -> Result<Event, serde_json::Error> {
    serde_json::from_value(json!({"id":"32345678-1234-4234-8234-123456789abc",
            "thread_id":"42345678-1234-4234-8234-123456789abc",
            "session_id":"12345678-1234-4234-8234-123456789abc",
            "turn_id":"22345678-1234-4234-8234-123456789abc",
            "created_at":"2026-09-15T00:00:00Z","data":data}))
}
#[test]
fn closed_terminal_requires_correlated_guard_and_completed_turn()
-> Result<(), Box<dyn std::error::Error>> {
    let mut events = vec![
        event(
            json!({"type":"messages","messages":[{"role":"assistant","content":"{\"action_id\":\"action-1\"}"}]}),
        )?,
        event(
            json!({"type":"custom","event_type":"sts2.exo-lookup-fetch-guard-v1",
                "payload":{"attempts":1,"forwarded":1,"denied":0,"tools":0}}),
        )?,
        event(json!({"type":"turn_ended"}))?,
    ];
    assert_eq!(validate_events(&events, &result()?, 0)?, "action-1");
    events.pop();
    assert!(validate_events(&events, &result()?, 0).is_err());
    events.push(event(json!({"type":"turn_ended"}))?);
    events.push(event(
        json!({"type":"error","message":"untrusted producer text"}),
    )?);
    assert!(validate_events(&events, &result()?, 0).is_err());
    assert!(wire::decode::<Action>(br#"{"action_id":"a","rationale":"extra"}"#).is_err());
    assert!(wire::decode::<Action>(br#"{"action_id":"a","action_id":"b"}"#).is_err());
    Ok(())
}
