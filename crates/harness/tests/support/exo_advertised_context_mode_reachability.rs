// SPDX-License-Identifier: MIT

//! The `context_modes` advertisement is a closed loop no request can exercise.
//!
//! #760's owner decision is option 1: the axis is KEPT and documented as build provenance. The
//! decision comment records the decisive fact — the descriptor and this guard are two reads of the
//! same constant, `SUPPORTED_CONTEXT_MODES`, so the axis is a build property published so a caller
//! can see what this one-shot build implements, not a gate that merely happens to be redundant.
//!
//! So `context_modes` describes what the build implements, not what a caller may request, and
//! `Continuity` is unreachable on the wire today. The same wording now appears in four places by
//! acceptance criterion 3: this doc comment, the `SUPPORTED_CONTEXT_MODES` constant, the
//! descriptor field, and the preflight guard. The two assertions below are UNCHANGED, and that is
//! the point rather than an omission: a documentation-only change does not alter wire reachability,
//! so the contract this module pins is still exactly true. If a later change adds a request-side
//! field or wires a reconstructed mode end to end, these assertions fail, which is the behaviour
//! #109 would need to revisit with its own evidence bar.
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

/// #760's acceptance criterion 3: the provenance decision is stated in the same words everywhere.
///
/// A decision recorded only in an issue comment decays the moment someone edits one doc comment
/// and leaves the other three reading the opposite story. This reads the four shipped sources and
/// requires the load-bearing clause in each, so a partial edit fails here instead of quietly
/// re-introducing the "caller may request this" reading the decision retired.
///
/// Non-vacuity: the paths are resolved relative to this file and every one is asserted to exist by
/// the read itself, so a rename cannot make this pass by skipping a site — a missing file fails
/// the read.
#[test]
fn the_context_axis_is_documented_as_build_provenance_in_every_place_that_names_it()
-> Result<(), String> {
    const CLAUSE: &str = "not what a caller may request";
    const CONTINUITY: &str = "unreachable on the wire";
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sites = [
        (
            "the advertised constant",
            root.join("src/exo_bridge_configuration/capability.rs"),
        ),
        (
            "the descriptor field",
            root.join("src/exo/contract/descriptor.rs"),
        ),
        (
            "the preflight guard",
            root.join("src/exo/contract/preflight.rs"),
        ),
        (
            "this module's own doc comment",
            root.join("tests/support/exo_advertised_context_mode_reachability.rs"),
        ),
    ];

    for (what, path) in sites {
        // An unreadable site returns `Err`, which fails the test carrying this message. The
        // enclosing test target allows `clippy::expect_used` but the workspace denies
        // `clippy::panic` and `clippy::unwrap_used`, so neither `panic!` nor `unwrap_or_else` is
        // available here.
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{what} at {} must be readable: {error}", path.display()))?;
        // Doc comments are hard-wrapped, so a clause can straddle a newline. Collapsing runs of
        // whitespace compares the sentences rather than the line breaks rustfmt chose.
        let flowed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flowed.contains(CLAUSE),
            "{what} ({}) no longer says the axis is `{CLAUSE}`; #760's recorded decision is that \
             all four sites carry the same words, and a one-site edit re-opens the ambiguity the \
             decision closed",
            path.display()
        );
        assert!(
            flowed.contains(CONTINUITY),
            "{what} ({}) no longer records that `Continuity` is `{CONTINUITY}` today",
            path.display()
        );
    }
    Ok(())
}
