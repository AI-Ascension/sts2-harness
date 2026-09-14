// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};

#[test]
fn conformance_vectors_execute_against_rust_contract() {
    let vectors: Value =
        serde_json::from_slice(CONFORMANCE).expect("conformance vectors are valid JSON");
    execute_request_vectors(
        vectors["request_vectors"]
            .as_array()
            .expect("request vectors"),
    );
    execute_decision_vectors(
        vectors["decision_vectors"]
            .as_array()
            .expect("decision vectors"),
    );
    execute_envelope_vectors(
        vectors["envelope_vectors"]
            .as_array()
            .expect("envelope vectors"),
    );
    execute_capability_vectors(
        vectors["capability_vectors"]
            .as_array()
            .expect("capability vectors"),
    );
}

fn execute_request_vectors(vectors: &[Value]) {
    for vector in vectors {
        let name = vector["name"].as_str().expect("request vector name");
        let expected = vector["expected"]
            .as_str()
            .expect("request vector expected");
        match name {
            "standard_at_bound" => {
                assert_eq!(expected, "accepted");
                assert_eq!(vector["bound"], json!(EXO_MAX_STANDARD_REQUEST_BYTES));
                let frame = pad_frame(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES);
                assert!(parse_bridge_request(&frame, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
            }
            "standard_over_bound" => {
                assert_eq!(expected, "too_large");
                assert_eq!(vector["bound"], json!(EXO_MAX_STANDARD_REQUEST_BYTES));
                let mut frame = pad_frame(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES);
                frame.push(b' ');
                assert_eq!(
                    parse_bridge_request(&frame, EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::TooLarge)
                );
                assert_eq!(
                    parse_bridge_request(&frame, EXO_MAX_MAP_REQUEST_BYTES),
                    Err(ExoWireError::TooLarge)
                );
            }
            "map_at_bound" => {
                assert_eq!(expected, "accepted");
                assert_eq!(vector["bound"], json!(EXO_MAX_MAP_REQUEST_BYTES));
                let frame = pad_frame(
                    &exo_contract_map::map_request_bytes(),
                    EXO_MAX_MAP_REQUEST_BYTES,
                );
                assert!(parse_bridge_request(&frame, EXO_MAX_MAP_REQUEST_BYTES).is_ok());
            }
            "map_over_bound" => {
                assert_eq!(expected, "too_large");
                assert_eq!(vector["bound"], json!(EXO_MAX_MAP_REQUEST_BYTES));
                let mut frame = pad_frame(
                    &exo_contract_map::map_request_bytes(),
                    EXO_MAX_MAP_REQUEST_BYTES,
                );
                frame.push(b' ');
                assert_eq!(
                    parse_bridge_request(&frame, EXO_MAX_MAP_REQUEST_BYTES),
                    Err(ExoWireError::TooLarge)
                );
            }
            "envelope_overhead_standard" => {
                assert_eq!(expected, "too_large");
                let frame = pad_frame(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES);
                assert_envelope_over_bound(
                    frame,
                    EXO_MAX_STANDARD_REQUEST_BYTES,
                    EXO_MAX_STANDARD_REQUEST_BYTES,
                );
                let frame = pad_frame(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES);
                assert_envelope_over_bound(
                    frame,
                    EXO_MAX_STANDARD_REQUEST_BYTES,
                    EXO_MAX_MAP_REQUEST_BYTES,
                );
            }
            "envelope_overhead_map" => {
                assert_eq!(expected, "too_large");
                let frame = pad_frame(
                    &exo_contract_map::map_request_bytes(),
                    EXO_MAX_MAP_REQUEST_BYTES,
                );
                assert_envelope_over_bound(
                    frame,
                    EXO_MAX_MAP_REQUEST_BYTES,
                    EXO_MAX_MAP_REQUEST_BYTES,
                );
            }
            "wrong_schema" => {
                assert_eq!(expected, "invalid_request");
                let mut wrong_schema: Value =
                    serde_json::from_slice(REQUEST).expect("golden request is JSON");
                wrong_schema["schema"] = json!("wrong-schema-v0");
                let wrong_schema =
                    serde_json::to_vec(&wrong_schema).expect("wrong schema serializes");
                assert_eq!(
                    parse_bridge_request(&wrong_schema, EXO_MAX_STANDARD_REQUEST_BYTES),
                    Err(ExoWireError::InvalidRequest)
                );
            }
            "swapped_package" => {
                assert_eq!(expected, "identity_mismatch");
                let mut trusted = ExoTrustedConfiguration {
                    identity: complete_identity(),
                    platform: ExoPlatform::LinuxX86_64,
                    profile: ExoProfile::Standard,
                    context_mode: ExoContextMode::Fresh,
                    limits: ExoLimits::reviewed(),
                };
                trusted.identity.package_digest = Some(String::from("9").repeat(64));
                let mut descriptor =
                    ExoCapabilityDescriptor::source_review().expect("source descriptor");
                enable_minimum_capabilities(&mut descriptor);
                descriptor.identity = complete_identity();
                assert_eq!(
                    preflight(&descriptor, &trusted),
                    Err(ExoPreflightError::IdentityMismatch("package_digest"))
                );
            }
            other => unreachable!("unhandled request conformance vector {other}"),
        }
    }
}

fn execute_decision_vectors(vectors: &[Value]) {
    for vector in vectors {
        let name = vector["name"].as_str().expect("decision vector name");
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
            "recovery" => {
                br#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"recover"}"#
                    .to_vec()
            }
            other => unreachable!("unhandled decision conformance vector {other}"),
        };
        assert!(
            parse_bridge_decision(&decision).is_ok(),
            "{name} decision rejected"
        );
    }
}

fn execute_envelope_vectors(vectors: &[Value]) {
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
}

fn execute_capability_vectors(vectors: &[Value]) {
    let base = ExoCapabilityDescriptor::source_review().expect("source descriptor");
    let identity = complete_identity();
    let trusted = ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        limits: ExoLimits::reviewed(),
    };
    for vector in vectors {
        let field = vector["field"].as_str().expect("capability vector field");
        assert_eq!(
            vector["downgrade"].as_str(),
            Some("unsupported"),
            "{field} must fail closed when unsupported"
        );
        let mut descriptor = base.clone();
        enable_minimum_capabilities(&mut descriptor);
        let required = match field {
            "evidence.terminal_decision" => {
                descriptor.evidence.terminal_decision = ExoCapabilityState::Unsupported;
                "evidence.terminal_decision"
            }
            "evidence.turn_identity" => {
                descriptor.evidence.turn_identity = ExoCapabilityState::Unsupported;
                "evidence.turn_identity"
            }
            "lifecycle.graceful_eof" => {
                descriptor.lifecycle.graceful_eof = ExoCapabilityState::Unsupported;
                "lifecycle.graceful_eof"
            }
            "lifecycle.idempotency" => {
                descriptor.lifecycle.idempotency = ExoCapabilityState::Unsupported;
                "lifecycle.idempotency"
            }
            "lifecycle.cancellation" => {
                descriptor.lifecycle.cancellation = ExoCapabilityState::Unsupported;
                "lifecycle.cancellation"
            }
            "lifecycle.recovery" => {
                descriptor.lifecycle.recovery = ExoCapabilityState::Unsupported;
                "lifecycle.recovery"
            }
            other => unreachable!("unhandled capability conformance vector {other}"),
        };
        assert_eq!(
            preflight_with_identity(descriptor, &trusted),
            Err(ExoPreflightError::RequiredCapability(required))
        );
    }
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
