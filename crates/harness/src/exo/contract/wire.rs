// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::strict::parse_strict_value;
use super::wire_types::MAX_CONTROL_ID_BYTES;
use super::{EXO_BRIDGE_WIRE_VERSION, EXO_DECISION_SCHEMA, EXO_MAX_RESPONSE_BYTES};
use crate::exo::protocol::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES};
use crate::exo::{Decision, ExoDecisionRequest};

pub use super::wire_types::{
    ExoBridgeDecisionEnvelope, ExoBridgeRequestEnvelope, ExoBridgeTurn, ExoControlIdentity,
    ExoTerminalOutcome, ExoWireError, ExoWireOutcome,
};

/// Parses one bounded UTF-8 request frame and re-runs the existing request validator.
pub fn parse_bridge_request(
    bytes: &[u8],
    max_request_bytes: usize,
) -> Result<ExoDecisionRequest, ExoWireError> {
    let maximum = max_request_bytes.min(EXO_MAX_MAP_REQUEST_BYTES);
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(ExoWireError::TooLarge);
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(ExoWireError::InvalidUtf8);
    }
    let value = parse_strict_value(bytes)?;
    let object = value.as_object().ok_or(ExoWireError::InvalidShape)?;
    const ALLOWED: [&str; 13] = [
        "schema",
        "provider_revision",
        "model_execution_id",
        "state_id",
        "generation",
        "observation",
        "legal_action_ids",
        "objective",
        "hard_constraints",
        "max_response_bytes",
        "map_context",
        "management_profile",
        "management_context",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ExoWireError::UnknownField);
    }
    let request = serde_json::from_value::<ExoDecisionRequest>(value)
        .map_err(|_| ExoWireError::InvalidRequest)?;
    let profile_limit = request_profile_limit(&request, maximum);
    if bytes.len() > profile_limit {
        return Err(ExoWireError::TooLarge);
    }
    request
        .encode(profile_limit)
        .map_err(|_| ExoWireError::InvalidRequest)?;
    Ok(request)
}

/// Encodes the selected bounded request/turn envelope.
pub fn encode_bridge_request(
    request_id: &str,
    turn_id: &str,
    request: &ExoDecisionRequest,
    max_request_bytes: usize,
) -> Result<Vec<u8>, ExoWireError> {
    validate_control_pair(request_id, turn_id)?;
    let maximum = max_request_bytes.min(EXO_MAX_MAP_REQUEST_BYTES);
    let profile_limit = request_profile_limit(request, maximum);
    request
        .encode(profile_limit)
        .map_err(|_| ExoWireError::InvalidRequest)?;
    let envelope = ExoBridgeRequestEnvelope {
        wire_version: EXO_BRIDGE_WIRE_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        turn_id: turn_id.to_owned(),
        request: request.clone(),
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| ExoWireError::InvalidRequest)?;
    if bytes.len() > profile_limit {
        return Err(ExoWireError::TooLarge);
    }
    Ok(bytes)
}

/// Parses one request/turn envelope and validates its inner STS2 request.
pub fn parse_bridge_request_envelope(
    bytes: &[u8],
    max_request_bytes: usize,
) -> Result<ExoBridgeRequestEnvelope, ExoWireError> {
    let maximum = max_request_bytes.min(EXO_MAX_MAP_REQUEST_BYTES);
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(ExoWireError::TooLarge);
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(ExoWireError::InvalidUtf8);
    }
    let value = parse_strict_value(bytes)?;
    let envelope = serde_json::from_value::<ExoBridgeRequestEnvelope>(value)
        .map_err(|_| ExoWireError::InvalidShape)?;
    if envelope.wire_version != EXO_BRIDGE_WIRE_VERSION {
        return Err(ExoWireError::VersionMismatch);
    }
    validate_control_pair(&envelope.request_id, &envelope.turn_id)?;
    let profile_limit = request_profile_limit(&envelope.request, maximum);
    if bytes.len() > profile_limit {
        return Err(ExoWireError::TooLarge);
    }
    envelope
        .request
        .encode(profile_limit)
        .map_err(|_| ExoWireError::InvalidRequest)?;
    Ok(envelope)
}

/// Parses exactly one terminal decision; correlation is carried by the outer control envelope.
pub fn parse_bridge_decision(bytes: &[u8]) -> Result<Decision, ExoWireError> {
    if bytes.is_empty() || bytes.len() > EXO_MAX_RESPONSE_BYTES {
        return Err(ExoWireError::TooLarge);
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(ExoWireError::InvalidUtf8);
    }
    let value = parse_strict_value(bytes)?;
    if !value.is_object() {
        return Err(ExoWireError::InvalidShape);
    }
    if value
        .get("schema")
        .and_then(Value::as_str)
        .is_some_and(|schema| schema != EXO_DECISION_SCHEMA)
    {
        return Err(ExoWireError::UnknownField);
    }
    crate::exo::parse_decision(bytes).map_err(ExoWireError::Decision)
}

/// Encodes a terminal response envelope without adding fields to the inner decision.
pub fn encode_bridge_response(
    request_id: &str,
    turn_id: &str,
    outcome: ExoWireOutcome,
    decision: Option<&[u8]>,
    error_code: Option<&str>,
) -> Result<Vec<u8>, ExoWireError> {
    validate_control_pair(request_id, turn_id)?;
    if error_code.is_some_and(|value| !valid_error_code(value)) {
        return Err(ExoWireError::InvalidShape);
    }
    let decision_value = decision.map(parse_strict_value).transpose()?;
    let valid_shape = match outcome {
        ExoWireOutcome::Decision => decision_value.is_some() && error_code.is_none(),
        ExoWireOutcome::Cancelled => decision_value.is_none() && error_code.is_none(),
        ExoWireOutcome::Failed => decision_value.is_none() && error_code.is_some(),
    };
    if !valid_shape {
        return Err(ExoWireError::InvalidShape);
    }
    if let Some(value) = &decision_value {
        if !value.is_object() {
            return Err(ExoWireError::InvalidShape);
        }
        let bytes = serde_json::to_vec(value).map_err(|_| ExoWireError::InvalidShape)?;
        crate::exo::parse_decision(&bytes).map_err(ExoWireError::Decision)?;
    }
    let envelope = ExoBridgeDecisionEnvelope {
        wire_version: EXO_BRIDGE_WIRE_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        turn_id: turn_id.to_owned(),
        outcome,
        decision: decision_value,
        error_code: error_code.map(str::to_owned),
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| ExoWireError::InvalidShape)?;
    if bytes.len() > EXO_MAX_RESPONSE_BYTES {
        return Err(ExoWireError::TooLarge);
    }
    Ok(bytes)
}

/// Parses a terminal response envelope and verifies request/turn correlation.
pub fn parse_bridge_decision_envelope(
    bytes: &[u8],
    expected_request_id: &str,
    expected_turn_id: &str,
) -> Result<Decision, ExoWireError> {
    if bytes.is_empty() || bytes.len() > EXO_MAX_RESPONSE_BYTES {
        return Err(ExoWireError::TooLarge);
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(ExoWireError::InvalidUtf8);
    }
    let value = parse_strict_value(bytes)?;
    let envelope = serde_json::from_value::<ExoBridgeDecisionEnvelope>(value)
        .map_err(|_| ExoWireError::InvalidShape)?;
    if envelope.wire_version != EXO_BRIDGE_WIRE_VERSION {
        return Err(ExoWireError::VersionMismatch);
    }
    validate_control_pair(&envelope.request_id, &envelope.turn_id)?;
    if envelope.request_id != expected_request_id || envelope.turn_id != expected_turn_id {
        return Err(ExoWireError::IdentityMismatch);
    }
    match envelope.outcome {
        ExoWireOutcome::Decision => {
            let decision = envelope.decision.ok_or(ExoWireError::InvalidShape)?;
            if envelope.error_code.is_some() {
                return Err(ExoWireError::InvalidShape);
            }
            let bytes = serde_json::to_vec(&decision).map_err(|_| ExoWireError::InvalidShape)?;
            parse_bridge_decision(&bytes)
        }
        ExoWireOutcome::Cancelled => {
            if envelope.decision.is_some() || envelope.error_code.is_some() {
                return Err(ExoWireError::InvalidShape);
            }
            Err(ExoWireError::Cancelled)
        }
        ExoWireOutcome::Failed => {
            if envelope.decision.is_some()
                || !envelope.error_code.as_deref().is_some_and(valid_error_code)
            {
                return Err(ExoWireError::InvalidShape);
            }
            Err(ExoWireError::RemoteFailure)
        }
    }
}

/// Verifies that a terminal receipt remains attached to the original host control tuple.
pub fn verify_control_identity(
    actual: &ExoControlIdentity,
    expected: &ExoControlIdentity,
) -> Result<(), ExoWireError> {
    actual.validate()?;
    expected.validate()?;
    if actual == expected {
        Ok(())
    } else {
        Err(ExoWireError::IdentityMismatch)
    }
}

fn valid_control_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONTROL_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn validate_control_pair(request_id: &str, turn_id: &str) -> Result<(), ExoWireError> {
    if valid_control_id(request_id) && valid_control_id(turn_id) {
        Ok(())
    } else {
        Err(ExoWireError::InvalidIdentity)
    }
}

fn request_profile_limit(request: &ExoDecisionRequest, maximum: usize) -> usize {
    let profile_limit = if request.map_context.is_some() {
        EXO_MAX_MAP_REQUEST_BYTES
    } else {
        EXO_MAX_STANDARD_REQUEST_BYTES
    };
    maximum.min(profile_limit)
}

fn valid_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
