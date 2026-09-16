// SPDX-License-Identifier: MIT

//! Reviewed admission boundary for the runtime Exo transport seam.
//!
//! The runtime must not give a model provider a transport until the offline, model-free contract
//! [`preflight`] has admitted the configured deployment. [`ExoAdmissionPlan`] assembles the
//! reviewed capability descriptor plus the operator-trusted configuration, refuses the run when a
//! required capability is missing or unverified, when a digest or revision does not match, or when
//! the advertised schema, runtime, platform or profile is unsupported, and only then admits one
//! correlated turn through [`ExoAdmittedTransport`]. Every refusal is produced before the inner
//! transport is dispatched, so a rejected deployment cannot cause a model or game effect.
//!
//! [`ExoRuntimeAdmission`] is the production-facing decision. `Enveloped` is the reviewed,
//! fail-closed default that speaks the versioned `sts2.exo-bridge-wire-v1` envelope. `Legacy` is
//! an explicit operator acknowledgement of the un-admitted raw-wire process bridge used by the
//! local `ollama`, `openai-astra` and `synthetic` fixtures, which cannot accept that envelope.

use crate::exo::{
    ExoCapabilityDescriptor, ExoIdentityError, ExoPreflightError, ExoTransport, ExoTransportError,
    ExoTrustedConfiguration, preflight,
};
use crate::exo_admitted_transport::{ExoAdmissionError, ExoAdmittedTransport};

/// Admission mode selected for the runtime Exo transport seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoAdmissionMode {
    /// Reviewed single-turn admission over the versioned request/decision envelope.
    Enveloped,
    /// Explicit acknowledgement of an un-admitted raw-wire process bridge.
    Legacy,
}

/// Offline admission refusal. Every variant is produced before any transport dispatch.
#[derive(Debug)]
pub enum ExoAdmissionRefusal {
    Descriptor(ExoIdentityError),
    Preflight(ExoPreflightError),
    Admission(ExoAdmissionError),
}

impl std::fmt::Display for ExoAdmissionRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Descriptor(error) => {
                write!(formatter, "reviewed Exo descriptor is invalid: {error}")
            }
            Self::Preflight(error) => write!(formatter, "Exo preflight refused the run: {error}"),
            Self::Admission(error) => write!(formatter, "Exo admission refused the turn: {error}"),
        }
    }
}

impl std::error::Error for ExoAdmissionRefusal {}

impl From<ExoAdmissionRefusal> for String {
    /// Formats a refusal with the production boundary's fail-closed prefix, for callers that
    /// report a startup failure as a message rather than as a typed error.
    fn from(refusal: ExoAdmissionRefusal) -> Self {
        format!("Exo admission refused before any model or game effect: {refusal}")
    }
}

/// One reviewed deployment, admitted offline as exactly one correlated turn.
pub struct ExoAdmissionPlan {
    trusted: ExoTrustedConfiguration,
    model_execution_id: String,
    request_id: String,
    turn_id: String,
}

impl ExoAdmissionPlan {
    #[must_use]
    pub fn new(
        trusted: ExoTrustedConfiguration,
        model_execution_id: String,
        request_id: String,
        turn_id: String,
    ) -> Self {
        Self {
            trusted,
            model_execution_id,
            request_id,
            turn_id,
        }
    }

    /// Builds the reviewed descriptor whose deployment identity axes are the operator-trusted
    /// values. Capability axes stay exactly as shipped by the source review; they are not asserted
    /// from, or on behalf of, the configured bridge.
    fn reviewed_descriptor(&self) -> Result<ExoCapabilityDescriptor, ExoAdmissionRefusal> {
        let mut descriptor =
            ExoCapabilityDescriptor::source_review().map_err(ExoAdmissionRefusal::Descriptor)?;
        descriptor.identity = self.trusted.identity.clone();
        Ok(descriptor)
    }

    /// Runs the offline contract preflight for this deployment without contacting Exo or a model.
    pub fn validate(&self) -> Result<(), ExoAdmissionRefusal> {
        preflight(&self.reviewed_descriptor()?, &self.trusted)
            .map(|_| ())
            .map_err(ExoAdmissionRefusal::Preflight)
    }

    /// Admits one correlated turn over `transport`, or refuses before any dispatch.
    pub fn admit<T: ExoTransport>(
        &self,
        transport: T,
    ) -> Result<ExoAdmittedTransport<T>, ExoAdmissionRefusal> {
        // Refuse before the admitted wrapper exists so a rejected deployment never dispatches.
        self.validate()?;
        let descriptor = self.reviewed_descriptor()?;
        ExoAdmittedTransport::new(
            transport,
            &descriptor,
            &self.trusted,
            self.model_execution_id.clone(),
            self.request_id.clone(),
            self.turn_id.clone(),
        )
        .map_err(ExoAdmissionRefusal::Admission)
    }
}

/// The transport the runtime hands to its provider after admission.
pub enum AdmittedExoRuntimeTransport<T> {
    Enveloped(Box<ExoAdmittedTransport<T>>),
    Legacy(T),
}

impl<T: ExoTransport> ExoTransport for AdmittedExoRuntimeTransport<T> {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        match self {
            Self::Enveloped(transport) => {
                transport.exchange(request, max_response_bytes, timeout_millis)
            }
            Self::Legacy(transport) => {
                transport.exchange(request, max_response_bytes, timeout_millis)
            }
        }
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        match self {
            Self::Enveloped(transport) => transport.close(),
            Self::Legacy(transport) => transport.close(),
        }
    }
}

/// The reviewed admission decision for one runtime run.
pub enum ExoRuntimeAdmission {
    Enveloped(Box<ExoAdmissionPlan>),
    Legacy,
}

impl ExoRuntimeAdmission {
    /// Admits the reviewed envelope path, or refuses before the run is allowed to continue.
    pub fn enveloped(plan: ExoAdmissionPlan) -> Result<Self, ExoAdmissionRefusal> {
        plan.validate()?;
        Ok(Self::Enveloped(Box::new(plan)))
    }

    /// Records the explicit, un-admitted raw-wire acknowledgement.
    #[must_use]
    pub fn legacy() -> Self {
        Self::Legacy
    }

    #[must_use]
    pub fn mode(&self) -> ExoAdmissionMode {
        match self {
            Self::Enveloped(_) => ExoAdmissionMode::Enveloped,
            Self::Legacy => ExoAdmissionMode::Legacy,
        }
    }

    /// Produces the transport the provider may use, refusing before any dispatch when the
    /// reviewed envelope admission cannot be established.
    pub fn admit<T: ExoTransport>(
        &self,
        transport: T,
    ) -> Result<AdmittedExoRuntimeTransport<T>, ExoAdmissionRefusal> {
        match self {
            Self::Enveloped(plan) => plan
                .admit(transport)
                .map(|admitted| AdmittedExoRuntimeTransport::Enveloped(Box::new(admitted))),
            Self::Legacy => Ok(AdmittedExoRuntimeTransport::Legacy(transport)),
        }
    }
}
