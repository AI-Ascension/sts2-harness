// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use serde_json::{Value, json};

use crate::exo::{
    EXO_SOURCE_REVISION, ExoDecisionKind, ExoIdentity, ExoLimits, ExoProfile, ExoTransport,
    ExoTransportError, ExoWireOutcome, encode_bridge_response, parse_bridge_request_envelope,
};

use super::strict_envelope::{Correlation, StrictAdmission, StrictEnvelope};

#[derive(Default)]
struct Calls {
    requests: Vec<Vec<u8>>,
    limits: Vec<(usize, u32)>,
    closes: usize,
}

struct RecordingTransport {
    calls: Rc<RefCell<Calls>>,
    response: Vec<u8>,
    fail: bool,
}

impl ExoTransport for RecordingTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        let mut calls = self.calls.borrow_mut();
        calls.requests.push(request.to_vec());
        calls.limits.push((max_response_bytes, timeout_millis));
        if self.fail {
            Err(ExoTransportError::Unavailable)
        } else {
            Ok(self.response.clone())
        }
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.calls.borrow_mut().closes += 1;
        Ok(())
    }
}

fn strict_transport(
    response: Vec<u8>,
    fail: bool,
) -> Result<(StrictEnvelope<RecordingTransport>, Rc<RefCell<Calls>>), String> {
    let calls = Rc::new(RefCell::new(Calls::default()));
    let identity = ExoIdentity::source_only(Some(String::from("test-model")))
        .map_err(|error| error.to_string())?;
    let admission = StrictAdmission {
        identity,
        profile: ExoProfile::Standard,
        limits: ExoLimits::reviewed(),
        decision_kinds: vec![ExoDecisionKind::Action],
    };
    let correlation = Correlation::new(
        String::from("execution-1"),
        String::from("request-7"),
        String::from("turn-9"),
    )
    .map_err(|error| error.to_string())?;
    let transport = StrictEnvelope::new(
        RecordingTransport {
            calls: calls.clone(),
            response,
            fail,
        },
        admission,
        correlation,
    );
    Ok((transport, calls))
}

fn request() -> Result<(Vec<u8>, String), String> {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../../protocol-artifact/exo-bridge-v1/golden/request.json"
    ))
    .map_err(|error| error.to_string())?;
    value["model_execution_id"] = json!("execution-1");
    value["provider_revision"] = json!(EXO_SOURCE_REVISION);
    let action_id = value["legal_action_ids"][0]
        .as_str()
        .ok_or_else(|| String::from("golden request has no legal action id"))?
        .to_owned();
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    Ok((bytes, action_id))
}

fn response(decision: Value) -> Result<Vec<u8>, String> {
    let decision = serde_json::to_vec(&decision).map_err(|error| error.to_string())?;
    encode_bridge_response(
        "request-7",
        "turn-9",
        ExoWireOutcome::Decision,
        Some(&decision),
        None,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn strict_envelope_forwards_one_correlated_request_and_returns_only_its_decision()
-> Result<(), String> {
    let (request, action_id) = request()?;
    let decision = json!({"decision":"action","action_id":action_id,"rationale":"safe"});
    let (mut transport, calls) = strict_transport(response(decision.clone())?, false)?;
    let output = transport
        .exchange(&request, 4096, 1000)
        .map_err(|error| format!("exchange refused: {error}"))?;
    let parsed: Value = serde_json::from_slice(&output).map_err(|error| error.to_string())?;
    assert_eq!(parsed, decision);
    let calls = calls.borrow();
    assert_eq!(calls.requests.len(), 1);
    assert_eq!(calls.limits, vec![(4096, 1000)]);
    let envelope = parse_bridge_request_envelope(&calls.requests[0], 128 * 1024)
        .map_err(|error| error.to_string())?;
    assert_eq!(envelope.request_id, "request-7");
    assert_eq!(envelope.turn_id, "turn-9");
    assert_eq!(envelope.request.model_execution_id, "execution-1");
    Ok(())
}

#[test]
fn a_ambiguous_inner_failure_consumes_the_fence_and_cannot_be_retried() -> Result<(), String> {
    let (request, _) = request()?;
    let (mut transport, calls) = strict_transport(Vec::new(), true)?;
    assert_eq!(
        transport.exchange(&request, 4096, 1000),
        Err(ExoTransportError::Unavailable)
    );
    assert_eq!(
        transport.exchange(&request, 4096, 1000),
        Err(ExoTransportError::Unavailable)
    );
    assert_eq!(calls.borrow().requests.len(), 1);
    Ok(())
}

#[test]
fn close_remains_idempotent_on_the_shared_fence() -> Result<(), String> {
    let (mut transport, calls) = strict_transport(Vec::new(), false)?;
    transport
        .close()
        .map_err(|error| format!("close refused: {error}"))?;
    transport
        .close()
        .map_err(|error| format!("repeat close refused: {error}"))?;
    assert_eq!(calls.borrow().closes, 1);
    Ok(())
}

#[test]
fn invalid_source_or_execution_identity_is_refused_before_transport() -> Result<(), String> {
    let (bytes, _) = request()?;
    let mut value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    value["provider_revision"] = json!("0".repeat(40));
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    let (mut transport, calls) = strict_transport(Vec::new(), false)?;
    assert_eq!(
        transport.exchange(&bytes, 4096, 1000),
        Err(ExoTransportError::MalformedResponse)
    );
    assert!(calls.borrow().requests.is_empty());
    Ok(())
}

#[test]
fn illegal_decision_action_and_wrong_correlation_are_refused_after_one_exchange()
-> Result<(), String> {
    let (request, action_id) = request()?;
    let illegal = json!({"decision":"action","action_id":"not-legal","rationale":"safe"});
    let (mut transport, calls) = strict_transport(response(illegal)?, false)?;
    assert_eq!(
        transport.exchange(&request, 4096, 1000),
        Err(ExoTransportError::MalformedResponse)
    );
    assert_eq!(calls.borrow().requests.len(), 1);

    let valid_decision = json!({
        "decision":"action","action_id":action_id,"rationale":"safe"
    });
    let valid_response = response(valid_decision.clone())?;
    let (mut valid_transport, valid_calls) = strict_transport(valid_response.clone(), false)?;
    let accepted = valid_transport
        .exchange(&request, 4096, 1000)
        .map_err(|error| format!("valid baseline response refused: {error}"))?;
    let accepted: Value = serde_json::from_slice(&accepted).map_err(|error| error.to_string())?;
    assert_eq!(accepted, valid_decision);
    assert_eq!(valid_calls.borrow().requests.len(), 1);

    let mut envelope: Value =
        serde_json::from_slice(&valid_response).map_err(|error| error.to_string())?;
    envelope["request_id"] = json!("other-request");
    let wrong_correlation = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    let (mut transport, calls) = strict_transport(wrong_correlation, false)?;
    assert_eq!(
        transport.exchange(&request, 4096, 1000),
        Err(ExoTransportError::MalformedResponse)
    );
    assert_eq!(calls.borrow().requests.len(), 1);
    Ok(())
}
