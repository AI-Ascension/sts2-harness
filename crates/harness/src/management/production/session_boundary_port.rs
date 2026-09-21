// SPDX-License-Identifier: MIT

//! The served write port of one held prepared-application approval.
//!
//! The port is the only place the held approval reaches the provider: it records the approved bytes
//! first and performs the exchange second, so the approved material and the bytes the boundary wrote
//! are one value. A sink that cannot record therefore stops the boundary before anything reaches the
//! provider, and the completion is recorded only once the exchange returns.

use super::super::session::{decision_provider_error, exchange_unresolved};
use crate::context_capture::{
    ApprovedDispatchMaterial, CaptureComponent, CapturePort, DispatchError, PreparedDispatchPort,
    PreparedInput,
};
use crate::context_control::PreparedContext;
use crate::episode::{DecisionInput, DecisionSource};
use crate::management::ManagementError;
use std::fmt;

/// The served write port: records the approved bytes, then writes them to the provider boundary.
pub(super) struct ServedBoundaryPort<'a> {
    provider: &'a mut (dyn DecisionSource + Send),
    input: &'a DecisionInput,
    decision_profile_ref: &'a str,
    context_ref: &'a str,
    prepared: &'a PreparedContext,
    capture: &'a mut dyn CapturePort,
    decision: Option<crate::Decision>,
    provider_error: Option<crate::episode::PolicyError>,
    /// Whether the provider exchange was attempted, so a failure is never reported as a refusal
    /// that nothing needs to be reconciled with.
    attempted: bool,
}

impl<'a> ServedBoundaryPort<'a> {
    /// Binds one served release to the port that will perform its exchange.
    pub(super) fn new(
        provider: &'a mut (dyn DecisionSource + Send),
        input: &'a DecisionInput,
        decision_profile_ref: &'a str,
        context_ref: &'a str,
        prepared: &'a PreparedContext,
        capture: &'a mut dyn CapturePort,
    ) -> Self {
        Self {
            provider,
            input,
            decision_profile_ref,
            context_ref,
            prepared,
            capture,
            decision: None,
            provider_error: None,
            attempted: false,
        }
    }

    /// The decision the port exchanged, if the release reached the provider.
    pub(super) fn take_decision(&mut self) -> Option<crate::Decision> {
        self.decision.take()
    }

    /// Classify a refused release against the boundary it actually reached.
    pub(super) fn release_error(&mut self, error: DispatchError) -> ManagementError {
        if let Some(provider_error) = self.provider_error.take() {
            return decision_provider_error(provider_error);
        }
        if self.attempted {
            return exchange_unresolved(super::refusal(error));
        }
        super::refusal(error)
    }
}

impl fmt::Debug for ServedBoundaryPort<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServedBoundaryPort")
            .field("execution_id", &self.input.execution_id.get())
            .field("attempted", &self.attempted)
            .finish_non_exhaustive()
    }
}

impl PreparedDispatchPort for ServedBoundaryPort<'_> {
    fn write_prepared(
        &mut self,
        material: ApprovedDispatchMaterial<'_>,
    ) -> Result<usize, DispatchError> {
        if !self.capture.enabled() {
            return Err(DispatchError::CaptureDisabled);
        }
        let components: Vec<CaptureComponent<'_>> = material
            .components
            .iter()
            .map(|component| CaptureComponent {
                kind: component.kind,
                ordinal: component.ordinal,
                media_type: component.media_type.as_str(),
                bytes: component.bytes(),
            })
            .collect();
        let [component] = components.as_slice() else {
            return Err(DispatchError::InvalidMaterial);
        };
        // The approval is compared with the bytes this boundary is about to write before either the
        // record or the exchange happens, so a second serialization can never be recorded as the
        // approved one.
        if component.bytes != self.prepared.provider_bytes() {
            return Err(DispatchError::InvalidMaterial);
        }
        let approved_bytes = component.bytes.len();
        self.capture
            .prepared_input(PreparedInput {
                execution_id: material.execution_id,
                attempt_id: material.attempt_id,
                boundary: material.boundary,
                components: &components,
            })
            .map_err(DispatchError::from)?;
        self.attempted = true;
        match self.provider.decide_prepared_for(
            self.input,
            self.decision_profile_ref,
            self.context_ref,
            self.prepared,
        ) {
            Ok(decision) => {
                self.decision = Some(decision);
                self.capture
                    .write_completed_at(
                        material.execution_id,
                        material.attempt_id,
                        material.boundary,
                    )
                    .map_err(DispatchError::from)?;
                Ok(approved_bytes)
            }
            Err(error) => {
                let _ = self.capture.write_unknown(
                    material.execution_id,
                    material.attempt_id,
                    "prepared_boundary_transport_unknown",
                    material.boundary,
                );
                self.provider_error = Some(error);
                Err(DispatchError::Indeterminate)
            }
        }
    }
}
