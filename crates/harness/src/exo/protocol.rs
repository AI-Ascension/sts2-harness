// SPDX-License-Identifier: MIT

mod request;

pub use request::ExoDecisionRequest;
use request::{request_from_prompt, valid_revision};

use crate::context_capture::{
    CaptureBoundary, CaptureInput, CapturePort, generated_capture_attempt_id,
};
use crate::episode::map::{MAP_CONTEXT_WIRE_FIXED_BYTES, MAX_SNAPSHOT_BYTES};
use crate::exo::decision::{DecisionError, parse_decision};
use crate::exo::sandbox::{SandboxError, SanitizedObservation};
use provider_impl::error_code;

/// Maximum serialized request for the ordinary Exo schema.
pub const EXO_MAX_STANDARD_REQUEST_BYTES: usize = 128 * 1024;

// A map request consists of one ordinary request, the schema-name delta, the map_context field,
// and the fixed map wrapper around the schema-bounded snapshot. These are the exact serialized
// bytes added by serde_json for the current wire shape.
const MAP_SCHEMA_DELTA_BYTES: usize =
    "sts2.exo-decision-map-v1".len() - "sts2.exo-decision-v1".len();
const MAP_CONTEXT_FIELD_BYTES: usize = ",\"map_context\":".len();
/// Serialized bytes added by the map schema and wrapper around the ordinary request body.
pub const EXO_MAP_REQUEST_OVERHEAD_BYTES: usize =
    MAP_SCHEMA_DELTA_BYTES + MAP_CONTEXT_FIELD_BYTES + MAP_CONTEXT_WIRE_FIXED_BYTES;

/// Maximum serialized request that can carry a schema-bounded complete map.
pub const EXO_MAX_MAP_REQUEST_BYTES: usize =
    EXO_MAX_STANDARD_REQUEST_BYTES + MAX_SNAPSHOT_BYTES + EXO_MAP_REQUEST_OVERHEAD_BYTES;

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
            || self.max_request_bytes > EXO_MAX_MAP_REQUEST_BYTES
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
    capture: Option<Box<dyn CapturePort>>,
    attempt_id: Option<String>,
}

impl<T> ExoProvider<T> {
    pub fn new(transport: T, config: ExoConfig) -> Self {
        Self {
            transport,
            config,
            closed: false,
            capture: None,
            attempt_id: None,
        }
    }

    /// Attaches an optional inspection sink. The sink is sideband-only and receives the same
    /// encoded bytes that the existing transport receives. Sink failures are ignored by design.
    #[must_use]
    pub fn with_capture(mut self, capture: Box<dyn CapturePort>) -> Self {
        self.capture = Some(capture);
        self
    }

    /// Records a trusted process-side attempt identifier without changing the provider payload.
    pub fn set_capture_attempt_id(&mut self, attempt_id: Option<String>) {
        self.attempt_id = attempt_id;
    }

    pub(super) fn capture_attempt_id(&self) -> Option<&str> {
        self.attempt_id.as_deref()
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
        let attempt_id = self
            .attempt_id
            .clone()
            .unwrap_or_else(|| generated_capture_attempt_id("exo"));
        self.capture_prepared_with_attempt(
            request.model_execution_id.as_str(),
            Some(attempt_id.as_str()),
            CaptureBoundary::ExoSessionRequest,
            &bytes,
        );
        let response = self.transport_exchange(&bytes);
        match response {
            Ok(response) => {
                self.capture_write_completed(
                    request.model_execution_id.as_str(),
                    Some(attempt_id.as_str()),
                    CaptureBoundary::ExoSessionRequest,
                );
                Ok(response)
            }
            Err(error) => {
                self.capture_write_unknown(
                    request.model_execution_id.as_str(),
                    Some(attempt_id.as_str()),
                    error_code(ExoError::from(error)),
                    CaptureBoundary::ExoSessionRequest,
                );
                Err(ExoError::from(error))
            }
        }
    }

    pub(super) fn capture_prepared_with_attempt(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        boundary: CaptureBoundary,
        bytes: &[u8],
    ) {
        if let Some(capture) = self.capture.as_mut() {
            let _ = capture.prepared(CaptureInput {
                execution_id,
                attempt_id,
                boundary,
                bytes,
            });
        }
    }

    pub(super) fn capture_write_completed(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        boundary: CaptureBoundary,
    ) {
        if let Some(capture) = self.capture.as_mut() {
            let _ = capture.write_completed_at(execution_id, attempt_id, boundary);
        }
    }

    pub(super) fn capture_write_unknown(
        &mut self,
        execution_id: &str,
        attempt_id: Option<&str>,
        code: &str,
        boundary: CaptureBoundary,
    ) {
        if let Some(capture) = self.capture.as_mut() {
            let _ = capture.write_unknown(execution_id, attempt_id, code, boundary);
        }
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

#[path = "protocol_provider.rs"]
mod provider_impl;
