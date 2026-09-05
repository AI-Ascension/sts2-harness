// SPDX-License-Identifier: MIT

mod request;

pub use request::ExoDecisionRequest;
use request::{request_from_prompt, valid_revision};

use crate::exo::decision::{DecisionError, parse_decision};
use crate::exo::sandbox::{SandboxError, SanitizedObservation};
use crate::provider::{ModelRequest, ModelResponse, ProviderPort};

/// External Exo transport failure; no gameplay fallback is attached to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoTransportError {
    Unavailable,
    Timeout,
    OversizedResponse,
    MalformedResponse,
}

/// Transport owned by the harness adapter. Exo implementation details stay outside this repo.
pub trait ExoTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError>;

    fn close(&mut self) -> Result<(), ExoTransportError>;
}

/// Reviewed and bounded adapter configuration. `revision` is mandatory and never inferred.
///
/// `forward_visible_seed` defaults to `true` so repeatable seeded runs retain their visible seed.
/// Callers can explicitly omit it for a seed-blind experiment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoConfig {
    pub revision: String,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub timeout_millis: u32,
    pub forward_visible_seed: bool,
}

impl ExoConfig {
    pub fn new(
        revision: impl Into<String>,
        max_request_bytes: usize,
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Self, ExoError> {
        let config = Self {
            revision: revision.into(),
            max_request_bytes,
            max_response_bytes,
            timeout_millis,
            forward_visible_seed: true,
        };
        config.validate()?;
        Ok(config)
    }

    /// Selects whether an experiment includes the host-visible seed in model requests.
    #[must_use]
    pub fn with_visible_seed_forwarding(mut self, enabled: bool) -> Self {
        self.forward_visible_seed = enabled;
        self
    }

    /// Applies the experiment's seed-visibility setting.
    pub(super) fn project(&self, observation: SanitizedObservation) -> SanitizedObservation {
        if self.forward_visible_seed {
            observation
        } else {
            observation.without_visible_seed()
        }
    }

    pub(super) fn validate(&self) -> Result<(), ExoError> {
        if !valid_revision(&self.revision)
            || self.max_request_bytes == 0
            || self.max_request_bytes > 128 * 1024
            || self.max_response_bytes == 0
            || self.max_response_bytes > 8 * 1024
            || self.timeout_millis == 0
            || self.timeout_millis > 120_000
        {
            return Err(ExoError::InvalidConfig);
        }
        Ok(())
    }
}

/// Adapter-level error retains unavailable/malformed distinctions for fail-closed callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoError {
    InvalidConfig,
    InvalidRequest,
    RequestTooLarge,
    Unavailable,
    Timeout,
    OversizedResponse,
    MalformedResponse,
    Decision(DecisionError),
    Sandbox(SandboxError),
    Closed,
}

impl std::fmt::Display for ExoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfig => "Exo adapter configuration is invalid or unpinned",
            Self::InvalidRequest => "Exo decision request is invalid",
            Self::RequestTooLarge => "Exo decision request exceeds its bound",
            Self::Unavailable => "Exo is unavailable",
            Self::Timeout => "Exo decision timed out",
            Self::OversizedResponse => "Exo response exceeds its bound",
            Self::MalformedResponse => "Exo response is malformed",
            Self::Decision(_) => "Exo decision failed strict validation",
            Self::Sandbox(_) => "observation failed the fair-play firewall",
            Self::Closed => "Exo adapter is closed",
        })
    }
}

impl std::error::Error for ExoError {}

impl From<DecisionError> for ExoError {
    fn from(error: DecisionError) -> Self {
        Self::Decision(error)
    }
}

impl From<SandboxError> for ExoError {
    fn from(error: SandboxError) -> Self {
        Self::Sandbox(error)
    }
}

impl From<ExoTransportError> for ExoError {
    fn from(error: ExoTransportError) -> Self {
        match error {
            ExoTransportError::Unavailable => Self::Unavailable,
            ExoTransportError::Timeout => Self::Timeout,
            ExoTransportError::OversizedResponse => Self::OversizedResponse,
            ExoTransportError::MalformedResponse => Self::MalformedResponse,
        }
    }
}

/// A small transport adapter that keeps Exo behind the existing provider port.
#[derive(Debug)]
pub struct ExoProvider<T> {
    transport: T,
    config: ExoConfig,
    closed: bool,
}

impl<T> ExoProvider<T> {
    pub fn new(transport: T, config: ExoConfig) -> Self {
        Self {
            transport,
            config,
            closed: false,
        }
    }

    #[must_use]
    pub fn config(&self) -> &ExoConfig {
        &self.config
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn execute_request(&mut self, request: ExoDecisionRequest) -> Result<Vec<u8>, ExoError>
    where
        T: ExoTransport,
    {
        if self.closed {
            return Err(ExoError::Closed);
        }
        self.config.validate()?;
        let bytes = request.encode(self.config.max_request_bytes)?;
        self.transport_exchange(&bytes).map_err(ExoError::from)
    }
}

impl<T: ExoTransport> ExoProvider<T> {
    pub(super) fn transport_exchange(
        &mut self,
        request: &[u8],
    ) -> Result<Vec<u8>, ExoTransportError> {
        if self.closed {
            return Err(ExoTransportError::Unavailable);
        }
        self.config
            .validate()
            .map_err(|_| ExoTransportError::MalformedResponse)?;
        let response = self.transport.exchange(
            request,
            self.config.max_response_bytes,
            self.config.timeout_millis,
        )?;
        if response.len() > self.config.max_response_bytes {
            return Err(ExoTransportError::OversizedResponse);
        }
        Ok(response)
    }

    pub(super) fn transport_close(&mut self) -> Result<(), ExoTransportError> {
        if self.closed {
            return Ok(());
        }
        let result = self.transport.close();
        if result.is_ok() {
            self.closed = true;
        }
        result
    }
}

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

fn error_code(error: ExoError) -> &'static str {
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
