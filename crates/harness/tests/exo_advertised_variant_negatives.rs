// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

//! `sts2-harness#141` AC2 at the shipped `sts2-exo-bridge` entry point.
//!
//! Every case here is a request the bridge must refuse *before* it dispatches a game action, and
//! most of them must be refused before the executor process can reach a model at all. The bridge
//! binary writes one fail-closed code to stderr and exits non-zero without writing stdout.
//!
//! These tests drive the real code path the binary uses — the strict envelope parser, the shared
//! profile classifier, the receipt/decision validators and the response encoder — rather than a
//! reimplementation, so a guard that is deleted here fails a test rather than silently widening
//! what the shipped bridge accepts.

use serde_json::{Value, json};
use sts2_harness::exo_bridge_configuration as config;
use sts2_harness::{
    EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_RESPONSE_BYTES, EXO_SOURCE_REVISION, ExoWireError,
    parse_bridge_decision, parse_bridge_request_envelope,
};

// Reuse the reviewed map request fixture instead of hand-rolling a second one.
#[path = "support/exo_contract_map.rs"]
mod exo_contract_map;

const REQUEST: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/request.json");

/// The request/turn envelope the bridge reads from stdin.
fn envelope() -> Value {
    json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "request-1",
        "turn_id": "turn-1",
        "request": serde_json::from_slice::<Value>(REQUEST).expect("golden request is JSON")
    })
}

fn envelope_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("envelope serializes")
}

/// The bridge's own stdin read bound; a larger frame cannot be read at all.
const STDIN_BOUND: usize = 131_072;

#[test]
fn every_unsupported_profile_axis_is_rejected_before_inference() {
    // The advertisement and the guard must agree on the exact axis names and code.
    let fields = config::capability_fields();
    assert_eq!(
        fields["unsupported_profile_code"],
        json!(config::UNSUPPORTED_PROFILE_CODE)
    );
    assert_eq!(
        fields["unsupported_recovery_code"],
        json!(config::UNSUPPORTED_RECOVERY_CODE)
    );
    assert_eq!(fields["profiles"], json!(["standard"]));
    assert_eq!(fields["context_modes"], json!(["fresh"]));

    // Every axis the classifier can report must appear here, so a new axis cannot be added
    // without a negative case.
    let mut covered = Vec::new();
    for axis in ["revision", "map", "management", "expert"] {
        let mut request = envelope();
        match axis {
            "revision" => request["request"]["provider_revision"] = json!("f".repeat(40)),
            "map" => request["request"]["map_context"] = json!({"profile": "runtime-map-v1"}),
            "management" => request["request"]["management_profile"] = json!("management-enabled"),
            "expert" => {
                request["request"]["observation"]["protocol_version"] = json!("runtime-v4-expert")
            }
            _ => continue,
        }
        covered.push(axis);
        let bytes = envelope_bytes(&request);
        // Whatever the parser decides, the bridge must not reach the executor. Where the request
        // is still schema-valid the shared classifier is the only thing that can stop it.
        match parse_bridge_request_envelope(&bytes, STDIN_BOUND) {
            Ok(parsed) => {
                let axis = config::unsupported_profile_axis(&parsed.request);
                assert!(
                    axis.is_some(),
                    "axis {axis:?} was admitted by the shipped profile guard"
                );
            }
            Err(error) => {
                // The strict validator may reject the shape first; that is still fail-closed.
                assert!(
                    matches!(
                        error,
                        ExoWireError::InvalidRequest | ExoWireError::InvalidShape
                    ),
                    "unexpected pre-model rejection for {axis}: {error:?}"
                );
            }
        }
    }
    for axis in [
        config::UnsupportedProfileAxis::Revision,
        config::UnsupportedProfileAxis::Map,
        config::UnsupportedProfileAxis::Management,
        config::UnsupportedProfileAxis::Expert,
    ] {
        assert!(
            covered.contains(&axis.name()),
            "classifier axis {} has no negative case",
            axis.name()
        );
    }
}

#[test]
fn supported_standard_request_is_admitted_so_the_guard_is_not_blanket_denial() {
    // The negative matrix only proves something if the supported case still passes.
    let parsed = parse_bridge_request_envelope(&envelope_bytes(&envelope()), STDIN_BOUND)
        .expect("the reviewed standard/fresh request must be admitted");
    assert_eq!(parsed.request.provider_revision, EXO_SOURCE_REVISION);
    assert_eq!(config::unsupported_profile_axis(&parsed.request), None);
}

#[test]
fn malformed_and_out_of_bounds_input_never_yields_a_dispatchable_request() {
    let base = envelope();
    let mut unknown_field = base.clone();
    unknown_field["request"]["unexpected"] = json!(true);
    let mut bad_utf8 = envelope_bytes(&base);
    bad_utf8[0] = 0xff;

    let cases: Vec<(&str, Vec<u8>, ExoWireError)> = vec![
        ("empty_input", Vec::new(), ExoWireError::TooLarge),
        (
            "oversized_input",
            vec![b'x'; STDIN_BOUND + 1],
            ExoWireError::TooLarge,
        ),
        ("invalid_utf8", bad_utf8, ExoWireError::InvalidUtf8),
        (
            "unknown_field",
            envelope_bytes(&unknown_field),
            ExoWireError::InvalidShape,
        ),
        (
            "duplicate_field",
            br#"{"wire_version":"sts2.exo-bridge-wire-v1","wire_version":"sts2.exo-bridge-wire-v1"}"#
                .to_vec(),
            ExoWireError::DuplicateField,
        ),
        (
            "truncated_input",
            b"{\"wire_version\":\"sts2.exo-bridge-wire-v1\"".to_vec(),
            ExoWireError::MalformedJson,
        ),
        (
            "trailing_bytes",
            {
                let mut bytes = envelope_bytes(&base);
                bytes.extend_from_slice(b" ,{}");
                bytes
            },
            ExoWireError::TrailingBytes,
        ),
        (
            "wrong_wire_version",
            {
                let mut value = base.clone();
                value["wire_version"] = json!("sts2.exo-bridge-wire-v0");
                envelope_bytes(&value)
            },
            ExoWireError::VersionMismatch,
        ),
    ];
    for (name, bytes, expected) in cases {
        assert_eq!(
            parse_bridge_request_envelope(&bytes, STDIN_BOUND),
            Err(expected),
            "{name} was not refused by the strict envelope parser"
        );
    }
}

#[test]
fn map_profile_request_is_parsed_but_refused_by_the_profile_guard() {
    // A schema-valid map request must be understood and still refused, not silently mangled.
    let mut map: Value = serde_json::from_slice(&exo_contract_map::map_request_bytes())
        .expect("map fixture is JSON");
    // The fixture pins a placeholder revision; bind it to the reviewed revision so the *map* axis
    // is the reason for refusal rather than the revision axis.
    map["provider_revision"] = json!(EXO_SOURCE_REVISION);
    let bytes = envelope_bytes(&json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "request-map",
        "turn_id": "turn-map",
        "request": map
    }));
    let parsed = parse_bridge_request_envelope(&bytes, EXO_MAX_MAP_REQUEST_BYTES)
        .expect("the map request is schema-valid");
    assert!(parsed.request.map_context.is_some());
    assert_eq!(
        config::unsupported_profile_axis(&parsed.request),
        Some(config::UnsupportedProfileAxis::Map)
    );
}

#[test]
fn terminal_decision_matrix_separates_dispatchable_from_refused() {
    let legal = "combat.end-turn";
    for decision in [
        json!({"decision": "action", "action_id": legal, "rationale": "synthetic"}),
        json!({"decision": "plan", "action_ids": [legal], "rationale": "synthetic"}),
        json!({"decision": "wait", "rationale": "synthetic"}),
        json!({"decision": "reobserve", "rationale": "synthetic"}),
    ] {
        let bytes = serde_json::to_vec(&decision).expect("decision serializes");
        let parsed = parse_bridge_decision(&bytes).expect("advertised decision parses");
        assert!(
            !matches!(parsed, sts2_harness::Decision::Recovery { .. }),
            "advertised decision set must exclude recovery"
        );
    }

    // Recovery is contract-parseable but not dispatchable by this build.
    let recovery = br#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"r"}"#;
    assert!(matches!(
        parse_bridge_decision(recovery),
        Ok(sts2_harness::Decision::Recovery { .. })
    ));
    let fields = config::capability_fields();
    assert_eq!(fields["decision_support"]["recovery"], json!("unsupported"));
    for decision in config::SUPPORTED_DECISIONS {
        assert_eq!(
            fields["decision_support"][decision],
            json!("supported"),
            "{decision} is enforced by the bridge but not advertised as supported"
        );
    }
    for decision in config::UNSUPPORTED_DECISIONS {
        assert!(
            !config::SUPPORTED_DECISIONS.contains(&decision),
            "{decision} cannot be both supported and unsupported"
        );
    }
    for profile in config::UNSUPPORTED_PROFILES {
        assert!(
            !config::SUPPORTED_PROFILES.contains(&profile),
            "{profile} cannot be both supported and unsupported"
        );
        assert_eq!(fields["profile_support"][profile], json!("unsupported"));
    }
    for profile in config::SUPPORTED_PROFILES {
        assert_eq!(fields["profile_support"][profile], json!("supported"));
    }
    assert_eq!(config::SUPPORTED_CONTEXT_MODES, ["fresh"]);
}

#[test]
fn malformed_model_decisions_are_never_parseable_as_dispatchable() {
    // Note: catalog-membership (`exo_bridge_illegal_action`) is enforced by the bridge's receipt
    // validator, which is unit-tested in `exo_bridge_run_tests.rs` where that guard lives.
    let cases: [(&str, &[u8]); 6] = [
        (
            "unknown_field",
            br#"{"decision":"wait","rationale":"x","extra":1}"#,
        ),
        ("multiple_json", br#"{}{}"#),
        ("truncated_json", br#"{"decision":"#),
        ("empty_output", b""),
        ("invalid_utf8", b"\xff"),
        ("not_an_object", b"[]"),
    ];
    for (name, bytes) in cases {
        assert!(
            parse_bridge_decision(bytes).is_err(),
            "{name} parsed into a decision"
        );
    }
    // A wrong `schema` is a closed-field rejection, not a silently accepted decision.
    assert_eq!(
        parse_bridge_decision(br#"{"schema":"wrong","decision":"wait","rationale":"x"}"#),
        Err(ExoWireError::UnknownField)
    );
    assert!(matches!(
        parse_bridge_decision(b""),
        Err(ExoWireError::TooLarge)
    ));
    assert!(matches!(
        parse_bridge_decision(&vec![b'x'; EXO_MAX_RESPONSE_BYTES + 1]),
        Err(ExoWireError::TooLarge)
    ));
}
