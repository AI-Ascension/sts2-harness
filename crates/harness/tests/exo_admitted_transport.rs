// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::{cell::RefCell, rc::Rc};

use serde_json::{Value, json};
use sts2_harness::*;

#[derive(Default)]
struct Recording {
    calls: Vec<(Vec<u8>, usize, u32)>,
    closes: usize,
}

struct Transport {
    recording: Rc<RefCell<Recording>>,
    response: Vec<u8>,
}

impl ExoTransport for Transport {
    fn exchange(
        &mut self,
        request: &[u8],
        maximum: usize,
        timeout: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.recording
            .borrow_mut()
            .calls
            .push((request.to_vec(), maximum, timeout));
        Ok(self.response.clone())
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.recording.borrow_mut().closes += 1;
        Ok(())
    }
}

fn configuration() -> (ExoCapabilityDescriptor, ExoTrustedConfiguration) {
    let mut descriptor = ExoCapabilityDescriptor::source_review().expect("descriptor");
    descriptor.identity = serde_json::from_value(json!({
        "source_revision": EXO_SOURCE_REVISION,
        "package_digest": "a".repeat(64), "extension_digest": "b".repeat(64),
        "bridge_digest": "c".repeat(64), "model_binding": "gpt-5-pro",
        "provider": "openai", "endpoint": "https://api.openai.com/v1",
        "prompt_digest": "d".repeat(64), "tool_digest": "e".repeat(64),
        "config_digest": "f".repeat(64), "contract_version": EXO_CONTRACT_VERSION,
        "native_instance_id": "fixture-native"
    }))
    .expect("synthetic identity");
    descriptor.lifecycle.cancellation = ExoCapabilityState::Supported;
    descriptor.lifecycle.recovery = ExoCapabilityState::Supported;
    descriptor.evidence.turn_identity = ExoCapabilityState::Supported;
    let trusted = ExoTrustedConfiguration {
        identity: descriptor.identity.clone(),
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: ExoRestrictedProfile::reviewed_private("/var/lib/sts2-harness/test"),
    };
    (descriptor, trusted)
}

fn request() -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/exo-bridge-v1/golden/request.json"
    ))
    .expect("project synthetic request");
    value["model_execution_id"] = json!("execution-1");
    value["provider_revision"] = json!(EXO_SOURCE_REVISION);
    value
}

fn response(decision: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "wire_version": EXO_BRIDGE_WIRE_VERSION, "request_id": "request-7",
        "turn_id": "turn-9", "outcome": "decision", "decision": decision,
        "error_code": null
    }))
    .expect("response")
}

fn admitted(response: Vec<u8>) -> (ExoAdmittedTransport<Transport>, Rc<RefCell<Recording>>) {
    let recording = Rc::new(RefCell::new(Recording::default()));
    let (descriptor, trusted) = configuration();
    let transport = ExoAdmittedTransport::new(
        Transport {
            recording: recording.clone(),
            response,
        },
        &descriptor,
        &trusted,
        "execution-1".into(),
        "request-7".into(),
        "turn-9".into(),
    )
    .expect("synthetic admission");
    (transport, recording)
}

#[test]
fn accepted_request_is_enveloped_once_and_returns_canonical_legacy_decision() {
    let decision = json!({"decision": "action",
        "action_id": request()["legal_action_ids"][0], "rationale": "safe"});
    let (mut transport, recording) = admitted(response(decision.clone()));
    let bytes = serde_json::to_vec(&request()).expect("request");
    let result = transport.exchange(&bytes, 4096, 1000).expect("accepted");
    assert_eq!(result, serde_json::to_vec(&decision).expect("canonical"));
    let state = recording.borrow();
    assert_eq!(state.calls.len(), 1);
    assert_eq!((state.calls[0].1, state.calls[0].2), (4096, 1000));
    let envelope =
        parse_bridge_request_envelope(&state.calls[0].0, 128 * 1024).expect("valid envelope");
    assert_eq!(envelope.request_id, "request-7");
    assert_eq!(envelope.turn_id, "turn-9");
    assert_eq!(envelope.request.model_execution_id, "execution-1");
    drop(state);
    assert_eq!(
        transport.exchange(&bytes, 4096, 1000),
        Err(ExoTransportError::Unavailable)
    );
    transport.close().expect("close");
    transport.close().expect("idempotent close");
    assert_eq!(recording.borrow().closes, 1);
}

#[test]
fn admission_denial_has_no_transport_effects() {
    let (descriptor, mut trusted) = configuration();
    trusted.profile = ExoProfile::Map;
    let recording = Rc::new(RefCell::new(Recording::default()));
    let result = ExoAdmittedTransport::new(
        Transport {
            recording: recording.clone(),
            response: vec![],
        },
        &descriptor,
        &trusted,
        "execution-1".into(),
        "request-7".into(),
        "turn-9".into(),
    );
    assert!(matches!(
        result,
        Err(ExoAdmissionError::Preflight(
            ExoPreflightError::ProfileUnsupported
        ))
    ));
    assert!(recording.borrow().calls.is_empty());
    assert_eq!(recording.borrow().closes, 0);
}

#[test]
fn invalid_revision_identity_request_and_budget_do_not_dispatch() {
    for (field, value) in [
        ("provider_revision", json!("a".repeat(40))),
        ("model_execution_id", json!("other-execution")),
        ("max_response_bytes", json!(8193)),
        ("unexpected", json!(true)),
        ("management_profile", json!("management-enabled")),
    ] {
        let (mut transport, recording) = admitted(vec![]);
        let mut request = request();
        request[field] = value;
        assert!(
            transport
                .exchange(&serde_json::to_vec(&request).expect("request"), 8192, 1000)
                .is_err()
        );
        assert!(recording.borrow().calls.is_empty(), "{field}");
    }
    let (mut transport, recording) = admitted(vec![]);
    assert!(transport.exchange(b"{}{}", 8192, 1000).is_err());
    assert!(
        transport
            .exchange(&serde_json::to_vec(&request()).expect("request"), 8192, 0)
            .is_err()
    );
    assert!(recording.borrow().calls.is_empty());
}

#[test]
fn malformed_multiple_wrong_correlation_and_illegal_decisions_never_retry() {
    let valid = response(json!({"decision": "wait", "rationale": "safe"}));
    let mut wrong: Value = serde_json::from_slice(&valid).expect("response");
    wrong["turn_id"] = json!("stale");
    let duplicate = String::from_utf8(valid.clone()).expect("utf8").replacen(
        "\"decision\":\"wait\"",
        "\"decision\":\"wait\",\"decision\":\"wait\"",
        1,
    );
    for response in [
        b"not json".to_vec(),
        [valid.clone(), valid].concat(),
        serde_json::to_vec(&wrong).expect("wrong"),
        duplicate.into_bytes(),
        response(json!({"decision": "action", "action_id": "illegal", "rationale": "safe"})),
        response(json!({"decision": "plan", "action_ids": ["illegal"], "rationale": "safe"})),
    ] {
        let (mut transport, recording) = admitted(response);
        let request = serde_json::to_vec(&request()).expect("request");
        assert!(transport.exchange(&request, 8192, 1000).is_err());
        assert_eq!(
            transport.exchange(&request, 8192, 1000),
            Err(ExoTransportError::Unavailable)
        );
        assert_eq!(recording.borrow().calls.len(), 1);
    }
}

#[test]
fn valid_expert_request_cannot_use_standard_admission() {
    let mut request = request();
    request["observation"] = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ))
    .expect("project synthetic expert observation");
    request["state_id"] = json!("live:7");
    request["generation"] = json!(7);
    request["legal_action_ids"] = json!([
        "play:7:card:1:enemy:1",
        "potion:7:potion:fire:enemy:1",
        "end:7"
    ]);
    let bytes = serde_json::to_vec(&request).expect("expert request");
    assert!(parse_bridge_request(&bytes, EXO_MAX_STANDARD_REQUEST_BYTES).is_ok());
    let (mut transport, recording) = admitted(vec![]);
    assert_eq!(
        transport.exchange(&bytes, 8192, 1000),
        Err(ExoTransportError::MalformedResponse)
    );
    assert!(recording.borrow().calls.is_empty());
}

#[test]
fn underlying_response_bound_is_checked_even_for_noncompliant_transport() {
    let (mut transport, recording) = admitted(vec![b' '; 257]);
    let request = serde_json::to_vec(&request()).expect("request");
    assert_eq!(
        transport.exchange(&request, 256, 200_000),
        Err(ExoTransportError::OversizedResponse)
    );
    assert_eq!(
        (recording.borrow().calls[0].1, recording.borrow().calls[0].2),
        (256, 120_000)
    );
}
