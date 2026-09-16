// SPDX-License-Identifier: MIT

use super::exo_contract_capability::execute_capability_vectors;
use super::*;
use serde_json::{Value, json};

#[test]
fn conformance_vectors_execute_against_rust_contract() {
    let vectors: Value =
        serde_json::from_slice(CONFORMANCE).expect("conformance vectors are valid JSON");
    let _ = execute_request_vectors(
        vectors["request_vectors"]
            .as_array()
            .expect("request vectors"),
    );
    let _ = execute_decision_vectors(
        vectors["decision_vectors"]
            .as_array()
            .expect("decision vectors"),
    );
    let _ = execute_envelope_vectors(
        vectors["envelope_vectors"]
            .as_array()
            .expect("envelope vectors"),
    );
    let _ = execute_capability_vectors(
        vectors["capability_vectors"]
            .as_array()
            .expect("capability vectors"),
    );
}

pub(super) fn execute_request_vectors(vectors: &[Value]) -> Vec<String> {
    let mut consumed = Vec::new();
    for vector in vectors {
        let name = vector["name"].as_str().expect("request vector name");
        consumed.push(name.to_owned());
        let expected = vector["expected"]
            .as_str()
            .expect("request vector expected");
        let fixture = vector["fixture"].as_str().expect("request vector fixture");
        if fixture == "generated:exo_contract_expert" {
            execute_expert_request_vector(vector);
            continue;
        }
        let source = fixture_bytes(fixture);
        if let Some(padding) = vector["padding"].as_str() {
            let bound = vector["bound"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .expect("padded request vector bound");
            let frame = apply_padding(&source, bound, padding);
            match expected {
                "accepted" => assert!(
                    parse_bridge_request(&frame, bound).is_ok(),
                    "{name} fixture rejected"
                ),
                "too_large" => {
                    assert_eq!(
                        parse_bridge_request(&frame, bound),
                        Err(ExoWireError::TooLarge),
                        "{name} fixture was not bounded"
                    );
                    if fixture == "golden/request.json" {
                        assert_eq!(
                            parse_bridge_request(&frame, EXO_MAX_MAP_REQUEST_BYTES),
                            Err(ExoWireError::TooLarge),
                            "{name} escaped its standard profile bound"
                        );
                    }
                }
                other => unreachable!("unhandled padded request result {other}"),
            }
            continue;
        }
        match expected {
            "too_large" => {
                let bound = vector["bound"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .expect("envelope vector bound");
                if source.len() > bound {
                    assert_eq!(
                        parse_bridge_request(&source, bound),
                        Err(ExoWireError::TooLarge),
                        "{name} fixture escaped its caller bound"
                    );
                    assert_envelope_over_bound(source.clone(), bound, bound);
                } else {
                    let frame = pad_frame(&source, bound);
                    assert_envelope_over_bound(frame, bound, bound);
                    if fixture == "golden/request.json" {
                        let frame = pad_frame(&source, bound);
                        assert_envelope_over_bound(frame, bound, EXO_MAX_MAP_REQUEST_BYTES);
                    }
                }
            }
            "invalid_request" => {
                let bound = vector["bound"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(EXO_MAX_STANDARD_REQUEST_BYTES);
                let mut wrong_schema: Value =
                    serde_json::from_slice(&source).expect("request fixture is JSON");
                wrong_schema["schema"] = json!("wrong-schema-v0");
                let wrong_schema =
                    serde_json::to_vec(&wrong_schema).expect("wrong schema serializes");
                assert_eq!(
                    parse_bridge_request(&wrong_schema, bound),
                    Err(ExoWireError::InvalidRequest),
                    "{name} expected invalid request"
                );
            }
            "identity_mismatch" => {
                let mut trusted = ExoTrustedConfiguration {
                    identity: complete_identity(),
                    platform: ExoPlatform::LinuxX86_64,
                    profile: ExoProfile::Standard,
                    context_mode: ExoContextMode::Fresh,
                    runtime: ExoRuntime::Responses,
                    limits: ExoLimits::reviewed(),
                    restricted: restricted_profile(),
                };
                trusted.identity.package_digest = Some(String::from("9").repeat(64));
                let mut descriptor =
                    ExoCapabilityDescriptor::source_review().expect("source descriptor");
                enable_minimum_capabilities(&mut descriptor);
                descriptor.identity = complete_identity();
                assert_eq!(
                    preflight(&descriptor, &trusted),
                    Err(ExoPreflightError::IdentityMismatch("package_digest")),
                    "{name} expected package identity mismatch"
                );
            }
            other => unreachable!("unhandled request result {other}"),
        }
    }
    consumed
}

fn execute_expert_request_vector(vector: &Value) {
    let name = vector["name"].as_str().expect("expert request vector name");
    let mutation = vector["mutation"]
        .as_str()
        .expect("expert request vector mutation");
    let parser_expected = vector["parser_expected"]
        .as_str()
        .expect("expert request parser expectation");
    let request = expert_request_variant(mutation);
    let bytes = serde_json::to_vec(&request).expect("expert request serializes");
    assert_eq!(
        parse_bridge_request(&bytes, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok(),
        parser_expected == "accepted",
        "{name} parser expectation was not met"
    );
}

fn fixture_bytes(fixture: &str) -> Vec<u8> {
    match fixture {
        "golden/request.json" => REQUEST.to_vec(),
        "generated:exo_contract_map" => exo_contract_map::map_request_bytes(),
        "golden/capability-source.json" => include_bytes!(
            "../../../../protocol-artifact/exo-bridge-v1/golden/capability-source.json"
        )
        .to_vec(),
        other => unreachable!("unhandled request fixture {other}"),
    }
}

fn apply_padding(source: &[u8], bound: usize, padding: &str) -> Vec<u8> {
    match padding {
        "utf8_space" => pad_frame(source, bound),
        "utf8_space_plus_one" => {
            let mut frame = pad_frame(source, bound);
            frame.push(b' ');
            frame
        }
        other => unreachable!("unhandled request padding {other}"),
    }
}

pub(super) fn execute_decision_vectors(vectors: &[Value]) -> Vec<String> {
    let mut consumed = Vec::new();
    for vector in vectors {
        let name = vector["name"].as_str().expect("decision vector name");
        consumed.push(name.to_owned());
        if vector["expected"].as_str() == Some("rejected") {
            let decision = match vector["decision"].as_str().expect("decision kind") {
                "unknown" => br#"{"decision":"teleport","rationale":"out of union"}"#.to_vec(),
                other => unreachable!("unhandled rejected decision conformance kind {other}"),
            };
            assert!(
                parse_bridge_decision(&decision).is_err(),
                "{name} out-of-union decision was accepted"
            );
            continue;
        }
        assert_eq!(
            vector["expected"].as_str(),
            Some("accepted"),
            "{name} must be an accepted terminal decision"
        );
        let decision = match vector["decision"].as_str().expect("decision kind") {
            "plan" => br#"{"decision":"plan","action_ids":["combat.end-turn"],"rationale":"plan"}"#
                .to_vec(),
            "action" => DECISION.to_vec(),
            "wait" => br#"{"decision":"wait","rationale":"wait"}"#.to_vec(),
            "reobserve" => br#"{"decision":"reobserve","rationale":"reobserve"}"#.to_vec(),
            "recovery" => match vector["recovery_kind"].as_str() {
                Some("reobserve") => {
                    br#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"recover"}"#
                        .to_vec()
                }
                Some("reconcile") => {
                    br#"{"decision":"recovery","recovery_kind":"reconcile","operation_id":"op-1","rationale":"recover"}"#
                        .to_vec()
                }
                Some("release_lease") => {
                    br#"{"decision":"recovery","recovery_kind":"release_lease","rationale":"recover"}"#
                        .to_vec()
                }
                Some("stop_episode") => {
                    br#"{"decision":"recovery","recovery_kind":"stop_episode","rationale":"recover"}"#
                        .to_vec()
                }
                other => unreachable!("unhandled recovery conformance kind {other:?}"),
            },
            other => unreachable!("unhandled decision conformance vector {other}"),
        };
        assert!(
            parse_bridge_decision(&decision).is_ok(),
            "{name} decision rejected"
        );
    }
    consumed
}

pub(super) fn execute_envelope_vectors(vectors: &[Value]) -> Vec<String> {
    let mut consumed = Vec::new();
    let failed = encode_bridge_response(
        "request-failed",
        "turn-failed",
        ExoWireOutcome::Failed,
        None,
        Some("remote_failure"),
    )
    .expect("failed response encodes");
    for vector in vectors {
        let name = vector["name"].as_str().expect("envelope vector name");
        consumed.push(name.to_owned());
        let expected = vector["expected"]
            .as_str()
            .expect("envelope vector expected");
        match name {
            "wrong_wire_version" => {
                assert_eq!(expected, "version_mismatch");
                let mut wrong_wire: Value =
                    serde_json::from_slice(&failed).expect("failed response is JSON");
                wrong_wire["wire_version"] = json!("wrong-wire-v0");
                let wrong_wire = serde_json::to_vec(&wrong_wire).expect("wrong wire serializes");
                assert_eq!(
                    parse_bridge_decision_envelope(&wrong_wire, "request-failed", "turn-failed"),
                    Err(ExoWireError::VersionMismatch)
                );
            }
            "wrong_request_id" => {
                assert_eq!(expected, "identity_mismatch");
                assert_eq!(
                    parse_bridge_decision_envelope(&failed, "other-request", "turn-failed"),
                    Err(ExoWireError::IdentityMismatch)
                );
            }
            "wrong_turn_id" => {
                assert_eq!(expected, "identity_mismatch");
                assert_eq!(
                    parse_bridge_decision_envelope(&failed, "request-failed", "other-turn"),
                    Err(ExoWireError::IdentityMismatch)
                );
            }
            "wrong_control_identity" => {
                assert_eq!(expected, "identity_mismatch");
                let expected_identity = control_identity("run-1");
                let mut actual = control_identity("run-1");
                actual.turn_id = String::from("turn-2");
                assert_eq!(
                    verify_control_identity(&actual, &expected_identity),
                    Err(ExoWireError::IdentityMismatch),
                    "{name} must reject a mismatched control identity"
                );
                let same = control_identity("run-1");
                assert!(
                    verify_control_identity(&same, &expected_identity).is_ok(),
                    "{name} control identity baseline is not self-consistent"
                );
            }
            "cancelled" => {
                assert_eq!(expected, "cancelled");
                let cancelled = encode_bridge_response(
                    "request-cancelled",
                    "turn-cancelled",
                    ExoWireOutcome::Cancelled,
                    None,
                    None,
                )
                .expect("cancelled response encodes");
                assert_eq!(
                    parse_bridge_decision_envelope(
                        &cancelled,
                        "request-cancelled",
                        "turn-cancelled"
                    ),
                    Err(ExoWireError::Cancelled)
                );
            }
            "failed" => {
                assert_eq!(expected, "remote_failure");
                assert_eq!(
                    parse_bridge_decision_envelope(&failed, "request-failed", "turn-failed"),
                    Err(ExoWireError::RemoteFailure)
                );
            }
            "duplicate_field" => {
                assert_eq!(expected, "duplicate_field");
                let duplicate =
                    br#"{"schema":"sts2.exo-decision-v1","schema":"sts2.exo-decision-v1"}"#;
                assert_eq!(
                    parse_bridge_request(duplicate, EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::DuplicateField)
                );
            }
            "nested_duplicate_field" => {
                assert_eq!(expected, "duplicate_field");
                let duplicate = br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","request":{"schema":"sts2.exo-decision-v1","schema":"sts2.exo-decision-v1"}}"#;
                assert_eq!(
                    parse_bridge_request_envelope(duplicate, EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::DuplicateField)
                );
            }
            "nested_unknown_field" => {
                assert_eq!(expected, "invalid_shape");
                let unknown = br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","request":{"schema":"sts2.exo-decision-v1","unknown":true}}"#;
                assert_eq!(
                    parse_bridge_request_envelope(unknown, EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::InvalidShape)
                );
            }
            "trailing_bytes" => {
                assert_eq!(expected, "trailing_bytes");
                let mut trailing = REQUEST.to_vec();
                trailing.extend_from_slice(b"{}");
                assert_eq!(
                    parse_bridge_request(&trailing, EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::TrailingBytes)
                );
            }
            "invalid_utf8" => {
                assert_eq!(expected, "invalid_utf8");
                assert_eq!(
                    parse_bridge_request(b"\xff", EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::InvalidUtf8)
                );
            }
            other => unreachable!("unhandled envelope conformance vector {other}"),
        }
    }
    consumed
}

fn assert_envelope_over_bound(frame: Vec<u8>, profile_limit: usize, caller_limit: usize) {
    let mut envelope =
        br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","request":"#
            .to_vec();
    envelope.extend_from_slice(&frame);
    envelope.push(b'}');
    assert!(envelope.len() > profile_limit);
    assert_eq!(
        parse_bridge_request_envelope(&envelope, caller_limit),
        Err(ExoWireError::TooLarge)
    );
}
