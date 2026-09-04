// SPDX-License-Identifier: MIT

use super::decision::{BoundDecision, Decision, DecisionError};
use super::protocol::{ExoDecisionRequest, ExoError, ExoProvider, ExoTransport};
use super::sandbox::SanitizedObservation;
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

    /// Sends only a sanitized observation and the complete current action ID set.
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
