// SPDX-License-Identifier: MIT

use crate::exo::{
    ExoCapabilityDescriptor, ExoPreflightError, ExoPreflightReport, ExoTransport,
    ExoTransportError, ExoTrustedConfiguration, ExoWireError, preflight,
};

#[path = "exo_admitted_transport/strict_envelope.rs"]
mod strict_envelope;

#[path = "exo_admitted_transport/synthetic_loopback.rs"]
mod synthetic_loopback;

#[cfg(test)]
#[path = "exo_admitted_transport/strict_envelope_tests.rs"]
mod strict_envelope_tests;

pub use synthetic_loopback::{
    SyntheticExoAdmissionError, SyntheticExoAdmissionPlan, SyntheticExoAdmissionReport,
    SyntheticExoAdmittedTransport,
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

/// One production-preflight-admitted execution over an existing transport.
///
/// The production HTTPS preflight remains the only constructor for this type. The shared strict
/// envelope core applies the same bounded request, one-shot, correlation and decision checks to
/// both this production route and the separate typed synthetic loopback route. A valid exchange
/// consumes this wrapper even if the peer fails; no retry or gameplay fallback is implicit. The
/// inner transport owns enforcement of the supplied deadline, which is not re-measured after a
/// successful response arrives.
pub struct ExoAdmittedTransport<T> {
    core: strict_envelope::StrictEnvelope<T>,
    report: ExoPreflightReport,
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
        let correlation =
            strict_envelope::Correlation::new(model_execution_id, request_id, turn_id)
                .map_err(ExoAdmissionError::Identity)?;
        let admission = strict_envelope::StrictAdmission {
            identity: report.identity.clone(),
            profile: report.profile,
            limits: report.limits.clone(),
            decision_kinds: descriptor.decision_kinds.clone(),
        };
        Ok(Self {
            core: strict_envelope::StrictEnvelope::new(inner, admission, correlation),
            report,
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
        self.core
            .exchange(bytes, max_response_bytes, timeout_millis)
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.core.close()
    }
}
