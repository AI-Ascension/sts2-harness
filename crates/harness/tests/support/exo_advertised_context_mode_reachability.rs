// SPDX-License-Identifier: MIT

//! The `context_modes` advertisement is a closed loop no request can exercise.
//!
//! `SUPPORTED_CONTEXT_MODES` and the capability descriptor's `context_modes` are both derived
//! rather than request-driven, and the axis is enforced at *preflight*, on a
//! [`sts2_harness::ExoTrustedConfiguration`] the bridge itself constructs. So the existing
//! negative matrix proves the preflight guard works, while the advertisement stays untested
//! against the only surface a caller actually holds: the request.
//!
//! What this pins is the honest current state, not a wish. The wire request has no
//! `context_mode` field at all, so a caller cannot ask for a non-fresh mode — which is the safe
//! direction. The gap is that nothing said so. If a later change adds the field to the request,
//! or wires a reconstructed mode end to end, this test is the thing that notices the
//! advertisement has started describing a capability the wire cannot reach (or, worse, that a
//! caller can now request one with no guard behind it).

use serde_json::{Value, json};
use sts2_harness::{ExoWireError, parse_bridge_request_envelope};

use super::{STDIN_BOUND, envelope};

/// The frame the parser is handed; the bridge reads exactly these bytes from stdin.
fn frame_bytes(request: &Value) -> Vec<u8> {
    let mut frame = envelope();
    frame["request"] = request.clone();
    serde_json::to_vec(&frame).expect("envelope serializes")
}

/// Every spelling of the context axis a caller might plausibly try, including the one the
/// bridge itself advertises as supported.
///
/// `fresh` is the load-bearing entry: it is the value `SUPPORTED_CONTEXT_MODES` publishes, so a
/// caller that reads the advertisement and dutifully echoes it back must still be refused. If that
/// ever parses, the advertisement is describing a request axis the guard does not cover.
fn attempted_axes() -> [(&'static str, Value); 3] {
    [
        ("context_mode", json!("fresh")),
        ("context_mode", json!("continuity")),
        ("context_modes", json!(["fresh"])),
    ]
}

/// A caller cannot reach the context-mode axis through the wire request, in any spelling.
///
/// Non-vacuity: the baseline request with no context field is asserted to parse in the companion
/// case below, so a blanket parser failure cannot make this pass for the wrong reason.
#[test]
fn the_wire_request_carries_no_context_mode_axis_to_reach() {
    for (field, value) in attempted_axes() {
        let mut request: Value = envelope()["request"].clone();
        request[field] = value.clone();

        assert_eq!(
            parse_bridge_request_envelope(&frame_bytes(&request), STDIN_BOUND),
            Err(ExoWireError::InvalidShape),
            "a request carrying {field}={value} was admitted, so the advertised context axis is \
             reachable from the wire and the preflight guard is no longer the only thing standing \
             between a caller and a non-fresh mode"
        );
    }
}

/// The refusals above must come from the closed request shape, not from an unrelated defect.
///
/// `InvalidShape` is a single code shared by every malformed frame, so without this the matrix
/// would also pass if the golden fixture simply stopped parsing. The reviewed standard/fresh
/// request is the control that separates "the axis is closed" from "the parser is broken".
#[test]
fn the_control_request_without_a_context_field_still_parses() {
    let request: Value = envelope()["request"].clone();
    let parsed = parse_bridge_request_envelope(&frame_bytes(&request), STDIN_BOUND)
        .expect("the reviewed request must parse, or the refusals above prove nothing");
    assert_eq!(
        parsed.request.provider_revision,
        sts2_harness::EXO_SOURCE_REVISION
    );
    // Restated so the control cannot drift into depending on the context axis at all.
    let advertised = sts2_harness::exo_bridge_configuration::SUPPORTED_CONTEXT_MODES;
    assert_eq!(advertised, ["fresh"]);
    let fields = sts2_harness::exo_bridge_configuration::capability_fields(
        &sts2_harness::exo_bridge_configuration::SUPPORTED_DECISIONS,
        &sts2_harness::exo_bridge_configuration::UNSUPPORTED_DECISIONS,
    );
    assert_eq!(fields["context_modes"], json!(["fresh"]));
}
