// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::json;
use sts2_harness::{
    EXO_BRIDGE_WIRE_VERSION, EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES,
    EXO_RESTRICTED_MAX_QUOTA_BYTES, EXO_RESTRICTED_MAX_RETENTION_DAYS, EXO_SOURCE_REVISION,
    ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoDecisionRequest,
    ExoDescriptorError, ExoIdentity, ExoLimits, ExoPlatform, ExoPreflightError,
    ExoPrivateStateError, ExoPrivateStatePolicy, ExoProfile, ExoRestrictedError, ExoRuntime,
    ExoTerminalOutcome, ExoToolCatalog, ExoToolCatalogError, ExoTrustedConfiguration, ExoWireError,
    ExoWireOutcome, PrivateRootKind, REVIEWED_MODEL_TOOLS, encode_bridge_request,
    encode_bridge_response, exo_bridge_manifest, parse_bridge_decision,
    parse_bridge_decision_envelope, parse_bridge_request, parse_bridge_request_envelope, preflight,
    responses_capable, responses_routing_capable, verify_control_identity,
    verify_exo_bridge_artifact,
};

#[path = "support/exo_contract_map.rs"]
mod exo_contract_map;

#[path = "support/exo_contract_expert.rs"]
mod exo_contract_expert;

#[path = "support/exo_contract_preflight.rs"]
mod exo_contract_preflight;

#[path = "support/exo_contract_schema.rs"]
mod exo_contract_schema;

#[path = "support/exo_contract_schema_vectors.rs"]
mod exo_contract_schema_vectors;

#[path = "support/exo_contract_projection.rs"]
mod exo_contract_projection;

#[path = "support/exo_contract_conformance.rs"]
mod exo_contract_conformance;

#[path = "support/exo_contract_capability.rs"]
mod exo_contract_capability;

#[path = "support/exo_contract_pin_inventory.rs"]
mod exo_contract_pin_inventory;

#[path = "support/exo_contract_restricted.rs"]
mod exo_contract_restricted;

#[path = "support/exo_contract_wire.rs"]
mod exo_contract_wire;

#[path = "support/exo_contract_helpers.rs"]
mod exo_contract_helpers;

use exo_contract_helpers::{
    complete_identity, control_identity, enable_minimum_capabilities, pad_frame,
    preflight_with_identity, restricted_profile,
};

const REQUEST: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/request.json");
const DECISION: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/decision-action.json");
const CONFORMANCE: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/conformance.json");
const SCHEMA: &[u8] = include_bytes!("../../../protocol-artifact/exo-bridge-v1/schema.json");
use exo_contract_expert::expert_request_variant;

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
fn preflight_is_pure_and_requires_every_deployment_identity() {
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    enable_minimum_capabilities(&mut descriptor);
    let complete = complete_identity();
    descriptor.identity = complete.clone();
    let trusted = ExoTrustedConfiguration {
        identity: complete,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: restricted_profile(),
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
    let mut base = base;
    enable_minimum_capabilities(&mut base);
    let identity = complete_identity();
    let trusted = ExoTrustedConfiguration {
        identity,
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: restricted_profile(),
    };

    let mut no_terminal = base.clone();
    no_terminal.evidence.terminal_decision = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_terminal, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "evidence.terminal_decision"
        ))
    );

    let mut no_turn_identity = base.clone();
    no_turn_identity.evidence.turn_identity = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_turn_identity, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "evidence.turn_identity"
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

    let mut no_idempotency = base.clone();
    no_idempotency.lifecycle.idempotency = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_idempotency, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "lifecycle.idempotency"
        ))
    );

    let mut no_cancellation = base.clone();
    no_cancellation.lifecycle.cancellation = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_cancellation, &trusted),
        Err(ExoPreflightError::RequiredCapability(
            "lifecycle.cancellation"
        ))
    );

    let mut no_recovery = base.clone();
    no_recovery.lifecycle.recovery = ExoCapabilityState::Unsupported;
    assert_eq!(
        preflight_with_identity(no_recovery, &trusted),
        Err(ExoPreflightError::RequiredCapability("lifecycle.recovery"))
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
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: restricted_profile(),
    };
    swapped_package.identity.package_digest = Some(String::from("9").repeat(64));
    let mut package_descriptor =
        ExoCapabilityDescriptor::source_review().expect("source descriptor is valid");
    enable_minimum_capabilities(&mut package_descriptor);
    package_descriptor.identity = complete_identity();
    assert_eq!(
        preflight(&package_descriptor, &swapped_package),
        Err(ExoPreflightError::IdentityMismatch("package_digest"))
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
        parse_bridge_decision_envelope(&response, "request-1", "other"),
        Err(ExoWireError::IdentityMismatch)
    );
    let nested_duplicate = br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","request":{"schema":"sts2.exo-decision-v1","schema":"sts2.exo-decision-v1"}}"#;
    assert_eq!(
        parse_bridge_request_envelope(nested_duplicate, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::DuplicateField)
    );
    let nested_unknown = br#"{"wire_version":"sts2.exo-bridge-wire-v1","request_id":"request-1","turn_id":"turn-1","request":{"schema":"sts2.exo-decision-v1","unknown":true}}"#;
    assert_eq!(
        parse_bridge_request_envelope(nested_unknown, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidShape)
    );
    let unicode_id = encode_bridge_request(
        "request-é",
        "turn-1",
        &request,
        EXO_MAX_STANDARD_REQUEST_BYTES,
    );
    assert_eq!(unicode_id, Err(ExoWireError::InvalidIdentity));
    let mut unicode_request: serde_json::Value =
        serde_json::from_slice(REQUEST).expect("golden request is JSON");
    unicode_request["objective"] = json!("survive-é");
    let unicode_request = serde_json::to_vec(&unicode_request).expect("unicode request serializes");
    assert_eq!(
        parse_bridge_request(&unicode_request, EXO_MAX_STANDARD_REQUEST_BYTES),
        Err(ExoWireError::InvalidRequest)
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
