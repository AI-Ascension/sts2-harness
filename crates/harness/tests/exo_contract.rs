// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::json;
use sts2_harness::{
    EXO_BRIDGE_WIRE_VERSION, EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES,
    EXO_SOURCE_REVISION, ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode,
    ExoControlIdentity, ExoDecisionRequest, ExoIdentity, ExoLimits, ExoPlatform, ExoPreflightError,
    ExoProfile, ExoTerminalOutcome, ExoTrustedConfiguration, ExoWireError, ExoWireOutcome,
    encode_bridge_request, encode_bridge_response, exo_bridge_manifest, parse_bridge_decision,
    parse_bridge_decision_envelope, parse_bridge_request, parse_bridge_request_envelope, preflight,
    verify_control_identity, verify_exo_bridge_artifact,
};

#[path = "support/exo_contract_map.rs"]
mod exo_contract_map;

const REQUEST: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/request.json");
const DECISION: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/decision-action.json");
const CONFORMANCE: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/conformance.json");
const SCHEMA: &[u8] = include_bytes!("../../../protocol-artifact/exo-bridge-v1/schema.json");

#[test]
fn frozen_artifact_and_source_descriptor_verify() {
    verify_exo_bridge_artifact().expect("source-derived artifact is internally consistent");
    assert!(exo_bridge_manifest().contains(EXO_SOURCE_REVISION));
    let descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor has valid identity");
    descriptor
        .validate()
        .expect("source descriptor is closed and valid");
    assert_eq!(
        descriptor.profile_support.map,
        ExoCapabilityState::Unverified
    );
    assert_eq!(
        descriptor.profile_support.expert,
        ExoCapabilityState::Unverified
    );
    assert_eq!(descriptor.context_modes, vec![ExoContextMode::Fresh]);
    assert_eq!(descriptor.platforms, vec![ExoPlatform::LinuxX86_64]);
}

#[test]
fn schema_and_conformance_vectors_are_closed_and_executable() {
    let schema: serde_json::Value =
        serde_json::from_slice(SCHEMA).expect("wire schema is valid JSON");
    let defs = schema
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .expect("wire schema has definitions");
    for name in [
        "request_envelope",
        "decision_envelope",
        "decision_request",
        "decision",
    ] {
        assert!(defs.contains_key(name), "missing schema definition {name}");
    }
    let conformance: serde_json::Value =
        serde_json::from_slice(CONFORMANCE).expect("conformance vectors are valid JSON");
    for (section, required) in [
        (
            "request_vectors",
            [
                "standard_at_bound",
                "standard_over_bound",
                "map_at_bound",
                "map_over_bound",
                "wrong_schema",
                "swapped_package",
            ]
            .as_slice(),
        ),
        (
            "decision_vectors",
            ["plan", "action", "wait", "reobserve", "recovery"].as_slice(),
        ),
        (
            "envelope_vectors",
            [
                "wrong_wire_version",
                "wrong_request_id",
                "cancelled",
                "failed",
                "duplicate_field",
                "trailing_bytes",
                "invalid_utf8",
            ]
            .as_slice(),
        ),
        (
            "capability_vectors",
            ["terminal_decision", "graceful_eof", "idempotency"].as_slice(),
        ),
    ] {
        let names = conformance
            .get(section)
            .and_then(serde_json::Value::as_array)
            .expect("conformance section is an array")
            .iter()
            .filter_map(|vector| vector.get("name").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        for name in required {
            assert!(names.contains(name), "{section} omitted {name}");
        }
    }
}

#[test]
fn preflight_is_pure_and_requires_every_deployment_identity() {
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    let complete = complete_identity();
    descriptor.identity = complete.clone();
    let trusted = ExoTrustedConfiguration {
        identity: complete,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        limits: ExoLimits::reviewed(),
    };
    let report = preflight(&descriptor, &trusted).expect("matching source/config admits");
    assert_eq!(report.model_calls, 0);
    assert_eq!(report.contract_version, "sts2-exo-bridge-v1");

    let mut incomplete = trusted.clone();
    incomplete.identity.bridge_digest = None;
    assert_eq!(
        preflight(&descriptor, &incomplete),
        Err(ExoPreflightError::MissingIdentity)
    );

    let mut map = trusted;
    map.profile = ExoProfile::Map;
    assert_eq!(
        preflight(&descriptor, &map),
        Err(ExoPreflightError::ProfileUnsupported)
    );
}

#[test]
fn preflight_downgrades_every_minimum_capability_and_rejects_unreviewed_pin() {
    let base = ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    let identity = complete_identity();
    let trusted = ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        limits: ExoLimits::reviewed(),
    };

    let mut no_terminal = base.clone();
    no_terminal.evidence.terminal_decision = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_terminal, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "evidence.terminal_decision"
        ))
    );

    let mut no_eof = base.clone();
    no_eof.lifecycle.graceful_eof = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_eof, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "lifecycle.graceful_eof"
        ))
    );

    let mut no_idempotency = base;
    no_idempotency.lifecycle.idempotency = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_idempotency, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "lifecycle.idempotency"
        ))
    );

    let mut wrong_pin = trusted;
    wrong_pin.identity.source_revision = String::from("a").repeat(40);
    let descriptor = ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    assert_eq!(
        preflight(&descriptor, &wrong_pin),
        Err(ExoPreflightError::UnreviewedSourceRevision)
    );

    let mut swapped_package = ExoTrustedConfiguration {
        identity: complete_identity(),
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        limits: ExoLimits::reviewed(),
    };
    swapped_package.identity.package_digest = Some(String::from("9").repeat(64));
    let mut package_descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    package_descriptor.identity = complete_identity();
    assert_eq!(
        preflight(&package_descriptor, &swapped_package),
        Err(ExoPreflightError::IdentityMismatch("package_digest"))
    );
}

#[test]
fn capability_descriptor_is_closed_and_identity_axes_are_distinct() {
    let descriptor = ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    let mut value = serde_json::to_value(descriptor).expect("descriptor serializes");
    value["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ExoCapabilityDescriptor>(value).is_err());
    let mut identity = complete_identity();
    identity.source_revision = String::from("a").repeat(40);
    assert_ne!(
        identity.source_revision,
        identity.bridge_digest.expect("digest exists")
    );
}

#[test]
fn strict_request_and_response_framing_rejects_bad_bytes() {
    let request = parse_bridge_request(REQUEST, EXO_MAX_STANDARD_REQUEST_BYTES)
        .expect("golden request is valid");
    let mut duplicate = REQUEST.to_vec();
    duplicate.extend_from_slice(br#" ,"extra":true}"#);
    assert!(matches!(
        parse_bridge_request(&duplicate, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::TrailingBytes | ExoWireError::MalformedJson)
    ));
    let duplicate = br#"{"schema":"sts2.exo-decision-v1","schema":"sts2.exo-decision-v1"}"#;
    assert_eq!(
        parse_bridge_request(duplicate, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::DuplicateField)
    );
    assert_eq!(
        parse_bridge_request(b"\xff", EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidUtf8)
    );
    let unknown = br#"{"schema":"sts2.exo-decision-v1","unknown":true}"#;
    assert_eq!(
        parse_bridge_request(unknown, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::UnknownField)
    );
    let envelope = encode_bridge_request(
        "request-1",
        "turn-1",
        &request,
        EXO_MAX_STANDARD_REQUEST_BYTES,
    )
    .expect("request envelope encodes");
    let decoded = parse_bridge_request_envelope(&envelope, EXO_MAX_STANDARD_REQUEST_BYTES)
        .expect("request envelope decodes");
    assert_eq!(decoded.request_id, "request-1");
    assert_eq!(decoded.turn_id, "turn-1");
    let response = encode_bridge_response(
        "request-1",
        "turn-1",
        ExoWireOutcome::Decision,
        Some(DECISION),
        None,
    )
    .expect("decision envelope encodes");
    assert!(parse_bridge_decision_envelope(&response, "request-1", "turn-1").is_ok());
    assert_eq!(
        parse_bridge_decision_envelope(&response, "other", "turn-1"),
        Err(ExoWireError::IdentityMismatch)
    );
    assert_eq!(
        parse_bridge_decision_envelope(
            &encode_bridge_response("request-1", "turn-1", ExoWireOutcome::Cancelled, None, None)
                .expect("cancel response encodes"),
            "request-1",
            "turn-1"
        ),
        Err(ExoWireError::Cancelled)
    );
    assert_eq!(EXO_BRIDGE_WIRE_VERSION, "sts2.exo-bridge-wire-v1");
}

#[test]
fn standard_and_map_boundaries_are_enforced() {
    assert!(parse_bridge_request(REQUEST, REQUEST.len()).is_ok());
    assert_eq!(
        parse_bridge_request(REQUEST, REQUEST.len() - 1),
        Err(ExoWireError::TooLarge)
    );

    let map = exo_contract_map::map_request_bytes();
    assert!(map.len() < EXO_MAX_MAP_REQUEST_BYTES);
    let decoded: ExoDecisionRequest =
        serde_json::from_slice(&map).expect("map request deserializes");
    assert_eq!(
        decoded
            .map_context
            .as_ref()
            .and_then(|value| value.get("profile").and_then(serde_json::Value::as_str)),
        Some("runtime-map-v1")
    );
    assert_eq!(
        decoded.map_context.as_ref().and_then(|value| value
            .get("schema_digest")
            .and_then(serde_json::Value::as_str)),
        Some(sts2_harness::RUNTIME_MAP_SCHEMA_DIGEST)
    );
    assert!(decoded.encode(EXO_MAX_MAP_REQUEST_BYTES).is_ok());
    let parsed_map = parse_bridge_request(&map, map.len());
    assert!(parsed_map.is_ok(), "map request rejected: {parsed_map:?}");
    assert_eq!(
        parse_bridge_request(&map, map.len() - 1),
        Err(ExoWireError::TooLarge)
    );
}

#[test]
fn every_terminal_decision_and_lifecycle_outcome_is_executable() {
    let decisions = [
        br#"{"decision":"plan","action_ids":["combat.end-turn"],"rationale":"plan"}"#.as_slice(),
        DECISION,
        br#"{"decision":"wait","rationale":"wait"}"#.as_slice(),
        br#"{"decision":"reobserve","rationale":"reobserve"}"#.as_slice(),
        br#"{"decision":"recovery","recovery_kind":"reobserve","rationale":"recover"}"#.as_slice(),
    ];
    for decision in decisions {
        assert!(parse_bridge_decision(decision).is_ok());
        let response = encode_bridge_response(
            "request-semantic",
            "turn-semantic",
            ExoWireOutcome::Decision,
            Some(decision),
            None,
        )
        .expect("semantic decision response encodes");
        assert!(
            parse_bridge_decision_envelope(&response, "request-semantic", "turn-semantic").is_ok()
        );
    }

    let failed = encode_bridge_response(
        "request-failed",
        "turn-failed",
        ExoWireOutcome::Failed,
        None,
        Some("remote_failure"),
    )
    .expect("failed response encodes");
    assert_eq!(
        parse_bridge_decision_envelope(&failed, "request-failed", "turn-failed"),
        Err(ExoWireError::RemoteFailure)
    );

    let mut wrong_wire: serde_json::Value =
        serde_json::from_slice(&failed).expect("failed response is JSON");
    wrong_wire["wire_version"] = json!("wrong-wire-v0");
    let wrong_wire = serde_json::to_vec(&wrong_wire).expect("wrong wire serializes");
    assert_eq!(
        parse_bridge_decision_envelope(&wrong_wire, "request-failed", "turn-failed"),
        Err(ExoWireError::VersionMismatch)
    );

    let mut wrong_schema: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    wrong_schema["schema"] = json!("wrong-schema-v0");
    let wrong_schema = serde_json::to_vec(&wrong_schema).expect("wrong schema serializes");
    assert_eq!(
        parse_bridge_request(&wrong_schema, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
    );
}

#[test]
fn control_receipts_bind_run_episode_and_turn_without_model_fields() {
    let identity = control_identity("run-1");
    let mut expected = identity.clone();
    expected.run_id = String::from("run-2");
    assert_eq!(
        verify_control_identity(&identity, &expected),
        Err(ExoWireError::IdentityMismatch)
    );
    let receipt = sts2_harness::ExoBridgeTurn {
        identity,
        outcome: ExoTerminalOutcome::Decision,
    };
    receipt
        .validate()
        .expect("control receipt identity is valid");
}

fn complete_identity() -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some(String::from("a").repeat(64)),
        extension_digest: Some(String::from("b").repeat(64)),
        bridge_digest: Some(String::from("c").repeat(64)),
        model_binding: Some(String::from("openai/model")),
        prompt_digest: Some(String::from("d").repeat(64)),
        tool_digest: Some(String::from("e").repeat(64)),
        config_digest: Some(String::from("f").repeat(64)),
        contract_version: String::from("sts2-exo-bridge-v1"),
        native_instance_id: Some(String::from("native-1")),
    }
}

fn control_identity(run_id: &str) -> ExoControlIdentity {
    ExoControlIdentity {
        run_id: run_id.to_owned(),
        episode_id: String::from("episode-1"),
        model_execution_id: String::from("execution-1"),
        agent_id: String::from("agent-1"),
        conversation_id: String::from("conversation-1"),
        session_id: String::from("session-1"),
        turn_id: String::from("turn-1"),
        idempotency_key: String::from("idem-1"),
    }
}

fn preflight_with_identity(
    descriptor: ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
) -> Result<sts2_harness::ExoPreflightReport, ExoPreflightError> {
    let mut descriptor = descriptor;
    descriptor.identity = trusted.identity.clone();
    preflight(&descriptor, trusted)
}
