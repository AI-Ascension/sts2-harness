// SPDX-License-Identifier: MIT
//! Negatives for the additive history pin, kept beside the wire they negotiate on.
//!
//! The wire module owns the frame contract and the bootstrap pin's own negatives live in
//! `lookup_bootstrap`; this module owns the pin history added.
use crate::lookup_wire::{self as wire, Frame, Payload};
use serde_json::json;

#[test]
fn history_payload_requires_its_own_additive_wire_version() -> Result<(), Box<dyn std::error::Error>>
{
    let frame = Frame {
        wire_version: wire::HISTORY_VERSION.into(),
        request_id: "request".into(),
        turn_id: "turn".into(),
        sequence: 1,
        payload: Payload::History {
            arguments: json!({"operation":"summary","operation_id":"history-1","branch_id":"branch"}),
        },
    };
    let bytes = serde_json::to_vec(&frame)?;
    assert_eq!(
        wire::version_for_payload(&frame.payload),
        wire::HISTORY_VERSION
    );
    assert!(wire::decode_frame(&bytes).is_ok());
    let mut legacy = serde_json::to_value(frame)?;
    legacy["wire_version"] = json!(wire::VERSION);
    assert!(wire::decode_frame(&serde_json::to_vec(&legacy)?).is_err());
    legacy["wire_version"] = json!(wire::BOOTSTRAP_VERSION);
    assert!(wire::decode_frame(&serde_json::to_vec(&legacy)?).is_err());
    // Selecting history widens the profile, not the pin: the bootstrap turn the shipped profile
    // could send is still admitted, but only on its own pin.
    let bootstrap_on_history = serde_json::to_vec(&Frame {
        wire_version: wire::HISTORY_VERSION.into(),
        request_id: "request".into(),
        turn_id: "turn".into(),
        sequence: 1,
        payload: Payload::Bootstrap {
            arguments: json!({}),
        },
    })?;
    assert!(wire::decode_frame(&bootstrap_on_history).is_err());
    let mut bootstrap_on_own_pin: serde_json::Value =
        serde_json::from_slice(&bootstrap_on_history)?;
    bootstrap_on_own_pin["wire_version"] = json!(wire::BOOTSTRAP_VERSION);
    assert!(wire::decode_frame(&serde_json::to_vec(&bootstrap_on_own_pin)?).is_ok());
    assert!(
        wire::feedback_for_version(
            &serde_json::to_vec(&Frame {
                wire_version: wire::VERSION.into(),
                request_id: "request".into(),
                turn_id: "turn".into(),
                sequence: 1,
                payload: Payload::Feedback {
                    value: json!({"history":{}}),
                },
            })?,
            "request",
            "turn",
            1,
            wire::HISTORY_VERSION,
        )
        .is_err()
    );
    Ok(())
}
