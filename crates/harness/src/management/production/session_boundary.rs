// SPDX-License-Identifier: MIT

//! Served release of the exact managed application boundary.
//!
//! [`crate::context_capture`] owns approval, holding, fences and receipts. This module is the
//! served caller that drives that protocol for the bytes a live managed decision is about to write,
//! so the boundary is not exercised only by its own tests. The recording write port is wired here:
//! the approved material is handed to it exactly once, it records what it wrote, and the provider
//! exchange happens only inside that port.
//!
//! The claim is deliberately narrow. The single recorded component is the encoded request the
//! harness itself writes to the Exo transport at `adapter.cli_input`; provider-internal
//! conversation, hidden context and the effective provider window are never claimed. A capture sink
//! that cannot record refuses the release before anything reaches the provider, so exactness is
//! never published for an unrecorded boundary.

use super::session::exchange_unresolved;
use super::*;
use crate::context_capture::{
    CaptureComponent, CaptureComponentKind, CapturePort, DispatchError, DispatchFences,
    DispatchReceipt, PreparedApplicationInput, PreparedDispatchController,
};
use crate::context_control::PreparedContext;
use crate::management::ContextRenderSourceIdentity;
use std::collections::BTreeMap;

#[path = "session_boundary_fences.rs"]
mod fences;
use fences::{BOUNDARY_ADAPTER_ID, boundary_attempt_id, boundary_fences};
#[path = "session_boundary_port.rs"]
mod port;
use port::ServedBoundaryPort;

/// Media type of the single component the managed Exo boundary writes.
const BOUNDARY_MEDIA_TYPE: &str = "application/json";

/// Recorded receipts for the approvals one served session already wrote.
///
/// A receipt is authoritative for its dispatch identity, so a repeated release of the same
/// invocation resolves to the retained receipt and never writes that approval a second time. Only
/// the receipt is retained: the approved bytes stay with the held approval that owns them, so a long
/// served run does not accumulate the material of every decision it already wrote.
#[derive(Debug, Default)]
pub(super) struct ServedBoundaryLedger {
    receipts: BTreeMap<String, DispatchReceipt>,
}

impl ServedBoundaryLedger {
    /// The receipt this served session already recorded for one dispatch identity.
    pub(super) fn receipt(&self, dispatch_id: &str) -> Option<&DispatchReceipt> {
        self.receipts.get(dispatch_id)
    }

    /// Retains one receipt under the dispatch identity it reconciles.
    pub(super) fn record(&mut self, receipt: DispatchReceipt) {
        self.receipts.insert(receipt.dispatch_id.clone(), receipt);
    }
}

/// A pre-exchange refusal of the served boundary.
///
/// Every refusal mapped here happens before the provider exchange, so the caller may treat it as a
/// clean refusal rather than an unresolved outcome.
fn refusal(error: DispatchError) -> ManagementError {
    match &error {
        DispatchError::UnsupportedAdapter
        | DispatchError::CaptureDisabled
        | DispatchError::Capture(_) => {
            ManagementError::capability("prepared_boundary_unsupported", error.to_string())
        }
        _ => ManagementError::conflict("prepared_boundary_refused", error.to_string()),
    }
}

/// The refusal of an approval this served session already wrote.
///
/// The recorded receipt proves the boundary was reached, so the outcome is unresolved: nothing new
/// may be written, and the caller reconciles against the retained receipt instead of retrying.
fn already_recorded() -> ManagementError {
    exchange_unresolved(ManagementError::unavailable(
        "prepared_boundary_already_recorded",
        "the served boundary already recorded this approval, so no new provider write exists",
    ))
}

/// A held approval for the exact application bytes of one served decision.
///
/// Nothing is written while the approval is drafted and committed: the bytes exist, the binding is
/// acknowledged and the approval stays held until [`Self::release`].
pub(super) struct ManagedBoundary {
    controller: PreparedDispatchController,
    dispatch_id: String,
    fences: DispatchFences,
}

impl ManagedBoundary {
    /// The deterministic dispatch identity of one served decision.
    ///
    /// The identity is derived from the served execution identity rather than a per-process counter,
    /// so a repeated release of the same invocation resolves to the approval it already wrote.
    pub(super) fn dispatch_id(execution_id: &str) -> String {
        format!("exo.{}", &crate::sha256_hex(execution_id.as_bytes())[..32])
    }

    /// Prepares and holds the exact bytes this decision is about to write.
    pub(super) fn hold(
        identity: &ContextRenderSourceIdentity,
        prepared: &PreparedContext,
        execution_id: &str,
    ) -> Result<Self, ManagementError> {
        let components = [CaptureComponent {
            kind: CaptureComponentKind::Stdin,
            ordinal: 0,
            media_type: BOUNDARY_MEDIA_TYPE,
            bytes: prepared.provider_bytes(),
        }];
        let approval = PreparedApplicationInput::prepare(
            BOUNDARY_ADAPTER_ID,
            execution_id,
            Some(&boundary_attempt_id(identity, execution_id)),
            &components,
        )
        .map_err(refusal)?;
        let fences = boundary_fences(identity, prepared.provider_revision());
        let dispatch_id = Self::dispatch_id(execution_id);
        let mut controller = PreparedDispatchController::new();
        controller
            .draft(&dispatch_id, approval, fences.clone())
            .map_err(refusal)?;
        controller.commit(&dispatch_id, &fences).map_err(refusal)?;
        Ok(Self {
            controller,
            dispatch_id,
            fences,
        })
    }

    /// Hands the held material to the recording write port exactly once.
    ///
    /// The port performs the provider exchange, so the approved bytes and the bytes written are one
    /// value. A retained receipt is authoritative: a repeated release returns it and never writes
    /// the same approval again, so no decision is `Some` on that path.
    pub(super) fn release(
        &mut self,
        provider: &mut (dyn DecisionSource + Send + '_),
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        prepared: &PreparedContext,
        capture: &mut dyn CapturePort,
    ) -> Result<(Option<crate::Decision>, DispatchReceipt), ManagementError> {
        let mut port = ServedBoundaryPort::new(
            provider,
            input,
            decision_profile_ref,
            context_ref,
            prepared,
            capture,
        );
        match self
            .controller
            .resume(&self.dispatch_id, &self.fences, &mut port)
        {
            Ok(receipt) => Ok((port.take_decision(), receipt)),
            Err(error) => Err(port.release_error(error)),
        }
    }
}

impl ProductionLiveWorkflowSession {
    /// Dispatches one managed decision through the held prepared boundary.
    ///
    /// The provider exchange happens inside the release, so a sink that cannot record, a fence that
    /// refuses, or an approval this session already wrote all stop the write entirely.
    pub(super) fn dispatch_managed_boundary(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        prepared: &PreparedContext,
        identity: &ContextRenderSourceIdentity,
        capture: &BoundaryCaptureSink,
    ) -> Result<crate::Decision, ManagementError> {
        let execution_id = input.execution_id.to_string();
        let dispatch_id = ManagedBoundary::dispatch_id(&execution_id);
        if self.boundary.receipt(&dispatch_id).is_some() {
            return Err(already_recorded());
        }
        let mut boundary = ManagedBoundary::hold(identity, prepared, &execution_id)?;
        let mut sink = capture.lock().map_err(|_| {
            ManagementError::unavailable(
                "prepared_boundary_capture_unavailable",
                "the served boundary capture sink is unavailable",
            )
        })?;
        let released = boundary.release(
            self.provider_mut()?,
            input,
            decision_profile_ref,
            context_ref,
            prepared,
            &mut **sink,
        );
        drop(sink);
        let (decision, receipt) = released?;
        self.boundary.record(receipt);
        decision.ok_or_else(already_recorded)
    }
}
