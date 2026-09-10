// SPDX-License-Identifier: MIT

use super::*;
use crate::provider::{ModelRequest, ModelResponse, ProviderPort};

impl<T: ExoTransport> ProviderPort for ExoProvider<T> {
    fn execute(
        &mut self,
        request: &ModelRequest,
    ) -> Result<ModelResponse, crate::error::ProviderError> {
        let decision_request = request_from_prompt(
            request.execution_id(),
            &self.config,
            request.prompt().as_str(),
        )
        .map_err(|error| provider_error(error_code(error), false))?;
        let output = self
            .execute_request(decision_request)
            .map_err(|error| provider_error(error_code(error), is_retryable(error)))?;
        parse_decision(&output)
            .map_err(|error| provider_error(decision_error_code(error), false))?;
        let output = String::from_utf8(output)
            .map_err(|_| provider_error("exo_malformed_response", false))?;
        let output = crate::provider::ModelOutput::new(output)
            .map_err(|_| provider_error("exo_oversized_response", false))?;
        ModelResponse::new(
            request.execution_id(),
            request.correlation().clone(),
            output,
        )
    }

    fn close(&mut self) -> Result<(), crate::error::PortError> {
        if !self.closed {
            self.transport_close().map_err(|_| {
                crate::error::PortError::new(
                    "exo_close_failed",
                    "Exo transport close failed",
                    false,
                )
            })?;
        }
        Ok(())
    }
}

fn provider_error(code: &'static str, retryable: bool) -> crate::error::ProviderError {
    crate::error::ProviderError::new(
        code,
        "Exo adapter rejected or could not complete the request",
        retryable,
    )
}

pub(super) fn error_code(error: ExoError) -> &'static str {
    match error {
        ExoError::Unavailable => "exo_unavailable",
        ExoError::Timeout => "exo_timeout",
        ExoError::OversizedResponse => "exo_oversized_response",
        ExoError::MalformedResponse | ExoError::Decision(_) => "exo_malformed_response",
        ExoError::Closed => "exo_closed",
        ExoError::InvalidConfig
        | ExoError::InvalidRequest
        | ExoError::RequestTooLarge
        | ExoError::Sandbox(_) => "exo_invalid_request",
    }
}

fn is_retryable(error: ExoError) -> bool {
    matches!(error, ExoError::Unavailable | ExoError::Timeout)
}

fn decision_error_code(error: DecisionError) -> &'static str {
    match error {
        DecisionError::TooLarge => "exo_oversized_response",
        DecisionError::InvalidJson
        | DecisionError::UnknownField
        | DecisionError::MissingField
        | DecisionError::InvalidValue
        | DecisionError::IllegalAction => "exo_malformed_response",
    }
}
