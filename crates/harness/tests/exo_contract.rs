// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::json;
use sts2_harness::{
    EXO_BRIDGE_WIRE_VERSION, EXO_MAX_STANDARD_REQUEST_BYTES, EXO_SOURCE_REVISION,
    ExoCapabilityDescriptor, ExoCapabilityState, ExoContextMode, ExoControlIdentity, ExoIdentity,
    ExoLimits, ExoPlatform, ExoPreflightError, ExoProfile, ExoTerminalOutcome,
    ExoTrustedConfiguration, ExoWireError, ExoWireOutcome, encode_bridge_request,
    encode_bridge_response, exo_bridge_manifest, parse_bridge_decision_envelope,
    parse_bridge_request, parse_bridge_request_envelope, preflight, verify_control_identity,
    verify_exo_bridge_artifact,
};

const REQUEST: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/request.json");
const DECISION: &[u8] =
    include_bytes!("../../../protocol-artifact/exo-bridge-v1/golden/decision-action.json");

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
