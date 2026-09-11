// SPDX-License-Identifier: MIT

use super::decision::{BoundDecision, Decision, DecisionError};
use super::protocol::{ExoDecisionRequest, ExoError, ExoProvider, ExoTransport};
use super::sandbox::SanitizedObservation;
use crate::episode::map::MapDecisionContext;
use crate::identity::ModelExecutionId;

/// One bounded Exo decision session. It has no heuristic action path.
#[derive(Debug)]
pub struct ExoSession<T> {
    provider: ExoProvider<T>,
    closed: bool,
}

impl<T> ExoSession<T> {
    #[must_use]
    pub fn new(provider: ExoProvider<T>) -> Self {
        Self {
            provider,
            closed: false,
        }
    }

    pub fn into_transport(self) -> T {
        self.provider.into_transport()
    }

    /// Adds the optional read-only capture sideband while leaving the provider payload unchanged.
    #[must_use]
    pub fn with_capture(mut self, capture: Box<dyn crate::context_capture::CapturePort>) -> Self {
        self.provider = self.provider.with_capture(capture);
        self
    }

    pub fn set_capture_attempt_id(&mut self, attempt_id: Option<String>) {
        self.provider.set_capture_attempt_id(attempt_id);
    }

    /// Sends only a sanitized observation and the complete current action ID set. The host
    /// `visible_seed` is removed unless `ExoConfig::forward_visible_seed` is set.
    #[allow(clippy::too_many_arguments)]
    pub fn decide(
        &mut self,
        execution_id: ModelExecutionId,
        state_id: impl Into<String>,
        generation: u64,
        observation: SanitizedObservation,
        legal_action_ids: Vec<String>,
        objective: impl Into<String>,
        constraints: Vec<String>,
    ) -> Result<Decision, ExoError>
    where
        T: ExoTransport,
    {
        if self.closed {
            return Err(ExoError::Closed);
        }
        self.provider.config().validate()?;
        let observation = self.provider.config().project(observation);
        let request = ExoDecisionRequest::new(
            execution_id,
            self.provider.config().revision.clone(),
            state_id,
            generation,
            observation,
            legal_action_ids,
            objective,
            constraints,
            self.provider.config().max_response_bytes,
        )?;
        let bytes = request.encode(self.provider.config().max_request_bytes)?;
        let attempt_id = self
            .provider
            .capture_attempt_id()
            .map(str::to_owned)
            .unwrap_or_else(|| crate::context_capture::generated_capture_attempt_id("exo"));
        self.provider.capture_prepared_with_attempt(
            &execution_id.to_string(),
            Some(attempt_id.as_str()),
            crate::context_capture::CaptureBoundary::ExoSessionRequest,
            &bytes,
        );
        let response = match self.provider.transport_exchange_for_session(&bytes) {
            Ok(response) => {
                self.provider.capture_write_completed(
                    &execution_id.to_string(),
                    Some(attempt_id.as_str()),
                    crate::context_capture::CaptureBoundary::ExoSessionRequest,
                );
                response
            }
            Err(error) => {
                self.provider.capture_write_unknown(
                    &execution_id.to_string(),
                    Some(attempt_id.as_str()),
                    match error {
                        super::protocol::ExoTransportError::Unavailable => "transport_unavailable",
                        super::protocol::ExoTransportError::Timeout => "transport_timeout",
                        super::protocol::ExoTransportError::OversizedResponse => {
                            "response_oversized"
                        }
                        super::protocol::ExoTransportError::MalformedResponse => {
                            "transport_malformed"
                        }
                    },
                    crate::context_capture::CaptureBoundary::ExoSessionRequest,
                );
                return Err(ExoError::from(error));
            }
        };
        super::decision::parse_decision(&response).map_err(ExoError::from)
    }

    /// Dispatches the exact immutable bytes approved by the management preview. The renderer
    /// has already bound the execution identity, adapter revision and selected content; this
    /// method refuses a different session identity before touching the transport.
    pub fn decide_prepared(
        &mut self,
        execution_id: ModelExecutionId,
        prepared: &crate::context_control::PreparedContext,
    ) -> Result<Decision, ExoError>
    where
        T: ExoTransport,
    {
        if self.closed {
            return Err(ExoError::Closed);
        }
        self.provider.config().validate()?;
        let expected_execution_id = execution_id.to_string();
        if prepared.provider_revision() != self.provider.config().revision
            || prepared.reserved_execution_id() != Some(expected_execution_id.as_str())
        {
            return Err(ExoError::InvalidRequest);
        }
        let bytes = prepared.provider_bytes();
        if bytes.is_empty() || bytes.len() > self.provider.config().max_request_bytes {
            return Err(ExoError::RequestTooLarge);
        }
        let attempt_id = self
            .provider
            .capture_attempt_id()
            .map(str::to_owned)
            .unwrap_or_else(|| crate::context_capture::generated_capture_attempt_id("exo"));
        self.provider.capture_prepared_with_attempt(
            &execution_id.to_string(),
            Some(attempt_id.as_str()),
            crate::context_capture::CaptureBoundary::ExoSessionRequest,
            bytes,
        );
        let response = match self.provider.transport_exchange_for_session(bytes) {
            Ok(response) => {
                self.provider.capture_write_completed(
                    &execution_id.to_string(),
                    Some(attempt_id.as_str()),
                    crate::context_capture::CaptureBoundary::ExoSessionRequest,
                );
                response
            }
            Err(error) => {
                self.provider.capture_write_unknown(
                    &execution_id.to_string(),
                    Some(attempt_id.as_str()),
                    "prepared_transport_unknown",
                    crate::context_capture::CaptureBoundary::ExoSessionRequest,
                );
                return Err(ExoError::from(error));
            }
        };
        super::decision::parse_decision(&response).map_err(ExoError::from)
    }

    /// Sends the same bounded decision request with a validated current map projection.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decide_with_map(
        &mut self,
        execution_id: ModelExecutionId,
        state_id: impl Into<String>,
        generation: u64,
        observation: SanitizedObservation,
        legal_action_ids: Vec<String>,
        objective: impl Into<String>,
        constraints: Vec<String>,
        map_context: MapDecisionContext,
    ) -> Result<Decision, ExoError>
    where
        T: ExoTransport,
    {
        if self.closed {
            return Err(ExoError::Closed);
        }
        self.provider.config().validate()?;
        let observation = self.provider.config().project(observation);
        let request = super::protocol::ExoDecisionRequest::new_with_map(
            execution_id,
            self.provider.config().revision.clone(),
            state_id,
            generation,
            observation,
            legal_action_ids,
            objective,
            constraints,
            self.provider.config().max_response_bytes,
            map_context,
        )?;
        let bytes = request.encode(self.provider.config().max_request_bytes)?;
        let response = self
            .provider
            .transport_exchange_for_session(&bytes)
            .map_err(ExoError::from)?;
        super::decision::parse_decision(&response).map_err(ExoError::from)
    }

    /// Binds an action directive to the current host catalog.
    pub fn bind_action(
        decision: Decision,
        legal_action_ids: &[String],
    ) -> Result<BoundDecision, DecisionError> {
        decision.bind(legal_action_ids)
    }

    pub fn close(&mut self) -> Result<(), ExoError>
    where
        T: ExoTransport,
    {
        if !self.closed {
            self.provider
                .transport_close_for_session()
                .map_err(ExoError::from)?;
            self.closed = true;
        }
        Ok(())
    }
}

// These narrow methods keep session policy separate from the ProviderPort implementation.
impl<T: ExoTransport> ExoProvider<T> {
    pub(super) fn transport_exchange_for_session(
        &mut self,
        request: &[u8],
    ) -> Result<Vec<u8>, super::protocol::ExoTransportError> {
        if request.len() > self.config().max_request_bytes {
            return Err(super::protocol::ExoTransportError::MalformedResponse);
        }
        self.transport_exchange(request)
    }

    pub(super) fn transport_close_for_session(
        &mut self,
    ) -> Result<(), super::protocol::ExoTransportError> {
        self.transport_close()
    }
}
