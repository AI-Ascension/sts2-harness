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

/// The golden envelope with the one field that carries `axis` set to an unsupported value.
///
/// Exhaustive on purpose: a new axis does not compile until it names the request shape it detects.
fn request_carrying(axis: config::UnsupportedProfileAxis) -> Value {
    use config::UnsupportedProfileAxis as Axis;
    let mut request = envelope();
    match axis {
        Axis::Revision => {
            request["request"]["provider_revision"] = json!("f".repeat(40));
        }
        Axis::Map => {
            request["request"]["map_context"] = json!({"profile": "runtime-map-v1"});
        }
        // The schema requires a non-null context for the enabled profile, so the request must carry
        // it to be schema-valid and reach the shared guard.
        Axis::Management => {
            request["request"]["management_profile"] = json!("management-enabled");
            request["request"]["management_context"] = json!({});
        }
        Axis::Expert => {
            request["request"]["observation"]["protocol_version"] = json!("runtime-v4-expert");
        }
    }
    request
}

#[test]
fn every_unsupported_profile_axis_is_rejected_before_inference() {
    // The advertisement and the guard must agree on the exact axis names and code.
    let fields =
        config::capability_fields(&config::SUPPORTED_DECISIONS, &config::UNSUPPORTED_DECISIONS);
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

    // Driven by `ALL`, the list the guard walks, and each case must report *that* axis.
    for axis in config::UnsupportedProfileAxis::ALL {
        let bytes = envelope_bytes(&request_carrying(axis));
        // Whatever the parser decides, the bridge must not reach the executor. Where the request
        // is still schema-valid the shared classifier is the only thing that can stop it.
        match parse_bridge_request_envelope(&bytes, STDIN_BOUND) {
            Ok(parsed) => assert_eq!(
                config::unsupported_profile_axis(&parsed.request),
                Some(axis),
                "axis {} was admitted by the shipped profile guard",
                axis.name()
            ),
            Err(error) => {
                // The strict validator may reject the shape first; that is still fail-closed.
                assert!(
                    matches!(
                        error,
                        ExoWireError::InvalidRequest | ExoWireError::InvalidShape
                    ),
                    "unexpected pre-model rejection for {}: {error:?}",
                    axis.name()
                );
            }
        }
    }
}

/// The advertisement must publish every profile axis the guard enforces, in both directions.
///
/// `profile_support` is the only way a caller can pre-check support, so an axis that is enforced
/// but absent here is undiscoverable except by triggering the deliberately identical rejection
/// code. This is the regression test for the `management` axis, which the guard enforced while the
/// advertisement omitted it: the guard and the advertisement were the same classifier, but the
/// published profile list was a separate constant that had fallen behind it.
///
/// The guard now walks [`config::UnsupportedProfileAxis::ALL`], which is also the advertisement's
/// source, so this asserts both directions of the only remaining pair: `ALL`/`UNSUPPORTED_PROFILES`.
#[test]
fn every_classifier_profile_axis_is_advertised() {
    let fields =
        config::capability_fields(&config::SUPPORTED_DECISIONS, &config::UNSUPPORTED_DECISIONS);
    let advertised = fields["profile_support"]
        .as_object()
        .expect("profile_support is an object");

    // Every classifier axis that names a request profile must appear as `unsupported`.
    for axis in config::UnsupportedProfileAxis::ALL {
        let Some(profile) = axis.profile_name() else {
            continue;
        };
        assert_eq!(
            advertised.get(profile),
            Some(&json!("unsupported")),
            "classifier axis {} is enforced but not advertised as unsupported",
            axis.name()
        );
        assert!(
            config::UNSUPPORTED_PROFILES.contains(&profile),
            "{profile} is enforced by the classifier but missing from UNSUPPORTED_PROFILES"
        );
    }

    // ...and the published list must not carry a profile no axis enforces.
    for profile in config::UNSUPPORTED_PROFILES {
        assert!(
            config::UnsupportedProfileAxis::ALL
                .iter()
                .any(|axis| axis.profile_name() == Some(profile)),
            "{profile} is advertised as unsupported but no classifier axis enforces it"
        );
    }

    // The advertised keys are exactly the supported profiles plus the enforced axes.
    let mut expected: Vec<&str> = config::SUPPORTED_PROFILES.to_vec();
    expected.extend(
        config::UnsupportedProfileAxis::ALL
            .iter()
            .filter_map(|axis| axis.profile_name()),
    );
    let mut observed: Vec<&str> = advertised.keys().map(String::as_str).collect();
    expected.sort_unstable();
    observed.sort_unstable();
    assert_eq!(observed, expected, "profile_support keys drifted");
    // Pinned as literals rather than derived, so shrinking `ALL` cannot silently widen the guard:
    // `management` was enforced-but-unadvertised once and must stay advertised, and no axis may be
    // published under a profile name it does not have.
    assert_eq!(observed, ["expert", "management", "map", "standard"]);
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
    let fields =
        config::capability_fields(&config::SUPPORTED_DECISIONS, &config::UNSUPPORTED_DECISIONS);
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

/// The lookup relay is terminal on an action id only, but it builds its advertisement from the
/// one-shot `description()`. If the decision fields were not re-projected, `lookup-describe` would
/// claim `plan`/`wait`/`reobserve` support on an entry point that cannot dispatch them — the exact
/// drift this change exists to stop. This drives the real methods rather than the helper, so
/// deleting the re-projection fails here.
#[test]
fn lookup_advertisement_does_not_borrow_the_one_shot_decision_set() {
    let loaded = config::Loaded {
        config: config::Configuration {
            schema: "sts2.exo-lookup-config-v1".to_owned(),
            executor: "/executor".into(),
            executor_sha256: "0".repeat(64),
            source_root: "/source".into(),
            extension: "/extension".into(),
            extension_sha256: "0".repeat(64),
            node: "/node".into(),
            node_sha256: "0".repeat(64),
            model: "o3-pro".to_owned(),
            endpoint: "http://127.0.0.1:8080".to_owned(),
        },
        digest: "0".repeat(64),
    };
    let lookup = loaded
        .lookup_description()
        .expect("the lookup advertisement is produced");
    let one_shot = loaded
        .description()
        .expect("the one-shot advertisement is produced");

    assert_eq!(lookup["schema"], json!("sts2.exo-lookup-capability-v1"));
    assert_eq!(lookup["decisions"], json!(["action_id"]));
    assert_eq!(lookup["decision_support"]["action_id"], json!("supported"));
    for decision in ["action", "plan", "wait", "reobserve", "recovery"] {
        assert_eq!(
            lookup["decision_support"][decision],
            json!("unsupported"),
            "the lookup relay advertised {decision} as supported"
        );
    }
    assert_ne!(lookup["decisions"], one_shot["decisions"]);
    for decision in config::LOOKUP_SUPPORTED_DECISIONS {
        assert!(
            !config::LOOKUP_UNSUPPORTED_DECISIONS.contains(&decision),
            "{decision} cannot be both supported and unsupported"
        );
    }
    // Profile advertisement is genuinely shared and must stay identical.
    assert_eq!(lookup["profiles"], one_shot["profiles"]);
    assert_eq!(lookup["profile_support"], one_shot["profile_support"]);
    assert_eq!(
        lookup["unsupported_profile_code"],
        one_shot["unsupported_profile_code"]
    );
    // The bootstrap profile inherits the same corrected decision advertisement.
    let bootstrap = loaded
        .lookup_bootstrap_description()
        .expect("the bootstrap advertisement is produced");
    assert_eq!(bootstrap["decisions"], json!(["action_id"]));
    assert_eq!(
        bootstrap["decision_support"]["reobserve"],
        json!("unsupported")
    );
}
