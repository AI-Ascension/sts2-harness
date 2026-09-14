// SPDX-License-Identifier: MIT

use crate::exo::{
    Decision, ExoBridgeDecisionEnvelope, ExoCapabilityDescriptor, ExoDecisionKind,
    ExoDecisionRequest, ExoPreflightError, ExoPreflightReport, ExoProfile, ExoTransport,
    ExoTransportError, ExoTrustedConfiguration, ExoWireError, ExoWireOutcome,
    encode_bridge_request, encode_bridge_response, parse_bridge_decision_envelope,
    parse_bridge_request, preflight,
};

/// Admission failure occurs before the underlying transport is invoked.
#[derive(Debug)]
pub enum ExoAdmissionError {
    Preflight(ExoPreflightError),
    Identity(ExoWireError),
}

impl std::fmt::Display for ExoAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preflight(error) => error.fmt(formatter),
            Self::Identity(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ExoAdmissionError {}

/// One admitted execution over an existing transport, with independent host-owned IDs.
///
/// Construction performs offline admission only. A valid exchange consumes this wrapper even
/// when the peer fails: no retry or gameplay fallback is implicit. The underlying transport owns
/// enforcement of the supplied deadline; this adapter also rejects late returned responses.
pub struct ExoAdmittedTransport<T> {
    inner: T,
    report: ExoPreflightReport,
    decision_kinds: Vec<ExoDecisionKind>,
    model_execution_id: String,
    request_id: String,
    turn_id: String,
    consumed: bool,
    closed: bool,
}

impl<T> ExoAdmittedTransport<T> {
    pub fn new(
        inner: T,
        descriptor: &ExoCapabilityDescriptor,
        trusted: &ExoTrustedConfiguration,
        model_execution_id: String,
        request_id: String,
        turn_id: String,
    ) -> Result<Self, ExoAdmissionError> {
        let report = preflight(descriptor, trusted).map_err(ExoAdmissionError::Preflight)?;
        // Reuse the contract's control-ID validator without creating a new identity grammar.
        encode_bridge_response(&request_id, &turn_id, ExoWireOutcome::Cancelled, None, None)
            .map_err(ExoAdmissionError::Identity)?;
        if model_execution_id.is_empty()
            || model_execution_id.len() > 512
            || !model_execution_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
        {
            return Err(ExoAdmissionError::Identity(ExoWireError::InvalidIdentity));
        }
        Ok(Self {
            inner,
            report,
            decision_kinds: descriptor.decision_kinds.clone(),
            model_execution_id,
            request_id,
            turn_id,
            consumed: false,
            closed: false,
        })
    }

    #[must_use]
    pub fn admission(&self) -> &ExoPreflightReport {
        &self.report
    }
}

impl<T: ExoTransport> ExoTransport for ExoAdmittedTransport<T> {
    fn exchange(
        &mut self,
        bytes: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        if self.closed || self.consumed {
            return Err(ExoTransportError::Unavailable);
        }
        let limits = &self.report.limits;
        let request_limit = match self.report.profile {
            ExoProfile::Map => limits.max_map_request_bytes,
            _ => limits.max_standard_request_bytes,
        } as usize;
        let request = parse_bridge_request(bytes, request_limit).map_err(wire_error)?;
        let profile = request_profile(&request);
        if request.model_execution_id != self.model_execution_id
            || request.provider_revision != self.report.identity.source_revision
            || profile != self.report.profile
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
        let envelope =
            encode_bridge_request(&self.request_id, &self.turn_id, &request, request_limit)
                .map_err(wire_error)?;
        self.consumed = true;
        let started = std::time::Instant::now();
        let response = self.inner.exchange(&envelope, response_limit, timeout)?;
        if started.elapsed() >= std::time::Duration::from_millis(u64::from(timeout)) {
            return Err(ExoTransportError::Timeout);
        }
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

impl<T> ExoAdmittedTransport<T> {
    fn decode_response(
        &self,
        request: &ExoDecisionRequest,
        response: &[u8],
    ) -> Result<Vec<u8>, ExoTransportError> {
        let decision = parse_bridge_decision_envelope(response, &self.request_id, &self.turn_id)
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
        if !self.decision_kinds.contains(&kind) {
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

fn request_profile(request: &ExoDecisionRequest) -> ExoProfile {
    if request
        .observation
        .get("protocol_version")
        .and_then(|v| v.as_str())
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
