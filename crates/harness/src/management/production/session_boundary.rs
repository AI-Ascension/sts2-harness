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
    DispatchLedger, DispatchReceipt, PreparedApplicationInput, PreparedDispatchController,
};
use crate::context_control::PreparedContext;
use crate::management::{ContextRenderSourceIdentity, ErrorClass};

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
///
/// The receipts are additionally carried through the attached [`ServedDispatchLedger`], so a
/// composition that rebuilds this session reloads the receipts its predecessor committed instead of
/// starting empty and writing an already-accepted boundary a second time.
#[derive(Debug, Default)]
pub(super) struct ServedBoundaryLedger {
    receipts: DispatchLedger,
    store: ServedDispatchLedger,
}

impl ServedBoundaryLedger {
    /// Rebuilds the receipts a restarted composition already committed.
    ///
    /// A store that cannot be read refuses the session rather than starting empty: an unavailable
    /// durable image is unknown, not empty, so the composition must never assume the boundary was
    /// not written.
    pub(super) fn restored(store: ServedDispatchLedger) -> Result<Self, ManagementError> {
        let mut ledger = Self {
            receipts: DispatchLedger::new(),
            store,
        };
        if let Some(image) = ledger.store.load().map_err(|_| store_unavailable())? {
            ledger.receipts = image.restore().map_err(|_| store_inconsistent())?;
        }
        Ok(ledger)
    }

    /// The receipt this served session already recorded for one dispatch identity.
    pub(super) fn receipt(&self, dispatch_id: &str) -> Option<&DispatchReceipt> {
        self.receipts.receipt(dispatch_id)
    }

    /// Retains one receipt under the dispatch identity it reconciles.
    pub(super) fn record(&mut self, receipt: DispatchReceipt) {
        self.receipts.record_receipt(receipt);
    }

    /// Makes every receipt this served session recorded durable.
    ///
    /// The recorded receipt is the evidence that the boundary was reached, so it is persisted before
    /// the release is reported: a composition that restarts then refuses a second write of the same
    /// approval instead of writing it again.
    pub(super) fn persist(&self) -> Result<(), ManagementError> {
        self.store
            .save(&self.receipts.durable())
            .map_err(|_| store_unavailable())
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

/// The refusal of a served session whose durable receipt store could not be read or written.
fn store_unavailable() -> ManagementError {
    ManagementError::unavailable(
        "prepared_boundary_ledger_unavailable",
        "the durable prepared-dispatch ledger is unavailable, so the recorded receipts are unknown",
    )
}

/// The refusal of a served session whose durable receipt store is not a consistent image.
fn store_inconsistent() -> ManagementError {
    ManagementError::unavailable(
        "prepared_boundary_ledger_inconsistent",
        "the durable prepared-dispatch ledger is inconsistent, so its receipts cannot be trusted",
    )
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

    /// The receipt `resume` recorded for this boundary, including an indeterminate outcome.
    ///
    /// A port whose transport outcome is indeterminate still records the receipt before `resume`
    /// returns, so this is how a lost reply is retained rather than dropped with the controller.
    pub(super) fn recorded_receipt(&self) -> Option<DispatchReceipt> {
        self.controller.ledger().receipt(&self.dispatch_id).cloned()
    }
}

impl ProductionLiveWorkflowSession {
    /// Retains one recorded receipt and makes the durable image current.
    ///
    /// The provider exchange may already have happened, so a store that cannot be written is
    /// reported as an unresolved outcome rather than a clean refusal.
    fn retain_receipt(&mut self, receipt: DispatchReceipt) -> Result<(), ManagementError> {
        self.boundary.record(receipt);
        self.boundary.persist().map_err(exchange_unresolved)
    }

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
        let mut sink = capture.lock().map_err(refusal)?;
        let released = boundary.release(
            self.provider_mut()?,
            input,
            decision_profile_ref,
            context_ref,
            prepared,
            &mut **sink,
        );
        drop(sink);
        // A receipt is retained and made durable whenever the write may already have reached the
        // provider, so a composition that restarts refuses a second write of the same approval
        // rather than writing it again.
        match released {
            Ok((decision, receipt)) => {
                self.retain_receipt(receipt)?;
                decision.ok_or_else(already_recorded)
            }
            Err(error) => {
                // A refusal that happened before the write leaves nothing to reconcile, so no
                // receipt is retained and the decision stays retryable. An unresolved outcome may
                // already have reached the provider, so the receipt `resume` recorded is retained
                // and made durable before the failure is reported.
                if error.class == ErrorClass::Unresolved
                    && let Some(receipt) = boundary.recorded_receipt()
                {
                    self.retain_receipt(receipt)?;
                }
                Err(error)
            }
        }
    }
}
