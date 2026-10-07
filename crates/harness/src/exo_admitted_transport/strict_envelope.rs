// SPDX-License-Identifier: MIT

use crate::exo::{
    Decision, ExoBridgeDecisionEnvelope, ExoDecisionKind, ExoDecisionRequest, ExoIdentity,
    ExoLimits, ExoProfile, ExoTransport, ExoTransportError, ExoWireError, ExoWireOutcome,
    encode_bridge_request, encode_bridge_response, parse_bridge_decision_envelope,
    parse_bridge_request,
};

pub(super) struct StrictAdmission {
    pub(super) identity: ExoIdentity,
    pub(super) profile: ExoProfile,
    pub(super) limits: ExoLimits,
    pub(super) decision_kinds: Vec<ExoDecisionKind>,
}

pub(super) struct Correlation {
    pub(super) model_execution_id: String,
    request_id: String,
    turn_id: String,
}

impl Correlation {
    pub(super) fn new(
        model_execution_id: String,
        request_id: String,
        turn_id: String,
    ) -> Result<Self, ExoWireError> {
        encode_bridge_response(&request_id, &turn_id, ExoWireOutcome::Cancelled, None, None)?;
        if model_execution_id.is_empty()
            || model_execution_id.len() > 512
            || !model_execution_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
        {
            return Err(ExoWireError::InvalidIdentity);
        }
        Ok(Self {
            model_execution_id,
            request_id,
            turn_id,
        })
    }
}

/// Shared closed wire fence. It contains no production-preflight or synthetic-route authority.
pub(super) struct StrictEnvelope<T> {
    inner: T,
    admission: StrictAdmission,
    correlation: Correlation,
    consumed: bool,
    closed: bool,
}

impl<T> StrictEnvelope<T> {
    pub(super) fn new(inner: T, admission: StrictAdmission, correlation: Correlation) -> Self {
        Self {
            inner,
            admission,
            correlation,
            consumed: false,
            closed: false,
        }
    }

    fn decode_response(
        &self,
        request: &ExoDecisionRequest,
        response: &[u8],
    ) -> Result<Vec<u8>, ExoTransportError> {
        let decision = parse_bridge_decision_envelope(
            response,
            &self.correlation.request_id,
            &self.correlation.turn_id,
        )
        .map_err(wire_error)?;
        let kind = match &decision {
            Decision::Action { action_id, .. } => {
                if !request.legal_action_ids.contains(action_id) {
                    return Err(ExoTransportError::MalformedResponse);
                }
                ExoDecisionKind::Action
            }
            Decision::Plan { action_ids, .. } => {
                if action_ids
                    .iter()
                    .any(|id| !request.legal_action_ids.contains(id))
                {
                    return Err(ExoTransportError::MalformedResponse);
                }
                ExoDecisionKind::Plan
            }
            Decision::Wait { .. } => ExoDecisionKind::Wait,
            Decision::Reobserve { .. } => ExoDecisionKind::Reobserve,
            Decision::Recovery { .. } => ExoDecisionKind::Recovery,
        };
        if !self.admission.decision_kinds.contains(&kind) {
            return Err(ExoTransportError::MalformedResponse);
        }
        let parsed: ExoBridgeDecisionEnvelope =
            serde_json::from_slice(response).map_err(|_| ExoTransportError::MalformedResponse)?;
        let value = parsed
            .decision
            .ok_or(ExoTransportError::MalformedResponse)?;
        serde_json::to_vec(&value).map_err(|_| ExoTransportError::MalformedResponse)
    }
}

impl<T: ExoTransport> ExoTransport for StrictEnvelope<T> {
    fn exchange(
        &mut self,
        bytes: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        if self.closed || self.consumed {
            return Err(ExoTransportError::Unavailable);
        }
        let limits = &self.admission.limits;
        let request_limit = match self.admission.profile {
            ExoProfile::Map => limits.max_map_request_bytes,
            _ => limits.max_standard_request_bytes,
        } as usize;
        let request = parse_bridge_request(bytes, request_limit).map_err(wire_error)?;
        let profile = request_profile(&request);
        if request.model_execution_id != self.correlation.model_execution_id
            || request.provider_revision != self.admission.identity.source_revision
            || profile != self.admission.profile
            || request.management_profile.is_some()
            || request.max_response_bytes > limits.max_response_bytes
            || max_response_bytes == 0
        {
            return Err(ExoTransportError::MalformedResponse);
        }
        let timeout = timeout_millis.min(limits.max_turn_time_millis);
        if timeout == 0 {
            return Err(ExoTransportError::Timeout);
        }
        let response_limit = max_response_bytes
            .min(request.max_response_bytes as usize)
            .min(limits.max_response_bytes as usize);
        let envelope = encode_bridge_request(
            &self.correlation.request_id,
            &self.correlation.turn_id,
            &request,
            request_limit,
        )
        .map_err(wire_error)?;
        self.consumed = true;
        let response = self.inner.exchange(&envelope, response_limit, timeout)?;
        if response.len() > response_limit {
            return Err(ExoTransportError::OversizedResponse);
        }
        self.decode_response(&request, &response)
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        if !self.closed {
            self.inner.close()?;
            self.closed = true;
        }
        Ok(())
    }
}

fn request_profile(request: &ExoDecisionRequest) -> ExoProfile {
    if request
        .observation
        .get("protocol_version")
        .and_then(|value| value.as_str())
        == Some("runtime-v4-expert")
    {
        ExoProfile::Expert
    } else if request.map_context.is_some() {
        ExoProfile::Map
    } else {
        ExoProfile::Standard
    }
}

fn wire_error(error: ExoWireError) -> ExoTransportError {
    match error {
        ExoWireError::RemoteFailure | ExoWireError::Cancelled => ExoTransportError::Unavailable,
        _ => ExoTransportError::MalformedResponse,
    }
}
