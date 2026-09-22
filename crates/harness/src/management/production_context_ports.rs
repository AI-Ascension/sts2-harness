// SPDX-License-Identifier: MIT

use super::*;
use crate::context_capture::{
    CaptureError, CaptureMode, CapturePort, DispatchError, DispatchLedgerError, DispatchLedgerPort,
    DurableDispatchLedger, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS, MemoryCapture, NoopCapture,
    NoopDispatchLedgerPort,
};
use crate::management::{
    ContextOwnerControlLimits, ContextRenderSource, ContextRenderSourceIdentity,
};
use std::sync::{Mutex, MutexGuard};

/// Receives the authoritative MCP observation and the run-reservation control
/// bound used by the served context owner. The owner composes its current
/// invocation binding before any delegated control effect.
pub trait LiveContextObservationPort: Send + Sync {
    fn record_observation(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        observation: &EpisodeObservation,
        control_limits: &ContextOwnerControlLimits,
    ) -> Result<(), ManagementError>;

    fn record_legal_actions(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), ManagementError>;

    fn invalidate(&self, actor: &AuthContext, request: &RunRequest, definition_digest: &str);
}

/// Resolves the authoritative encrypted source used by one actual provider
/// invocation and rechecks its owner-issued fence before and after inference.
pub trait LiveContextRenderPort: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn render_source_for_decision(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
    ) -> Result<ContextRenderSource, ManagementError>;

    #[allow(clippy::too_many_arguments)]
    fn assert_render_source_current(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
        binding: &RuntimeAuthorityBinding,
        control_limits: &ContextOwnerControlLimits,
        input: &DecisionInput,
        context_ref: &str,
        expected: &ContextRenderSourceIdentity,
    ) -> Result<(), ManagementError>;

    fn render_required(&self) -> bool;
}

/// The recording sink attached to the served boundary.
///
/// The sink is the served boundary's write port: the approved material is handed to it exactly once
/// and it records what the boundary wrote, so the approved manifest and the observed one are the
/// same value. The default sink cannot record, so a composition that attaches no sink refuses the
/// managed release before anything reaches the provider instead of publishing exactness for a
/// boundary nothing recorded.
#[derive(Clone, Debug)]
pub struct BoundaryCaptureSink(Arc<Mutex<Box<dyn CapturePort>>>);

impl BoundaryCaptureSink {
    /// Attaches one recording sink.
    pub fn new(sink: Box<dyn CapturePort>) -> Self {
        Self(Arc::new(Mutex::new(sink)))
    }

    /// A sink that cannot record: the served boundary refuses before any provider write.
    pub fn disabled() -> Self {
        Self::new(Box::new(NoopCapture))
    }

    /// The recording sink the served composition attaches.
    ///
    /// It is a bounded in-memory ring: the served boundary records the exact approved bytes and
    /// their lifecycle before the provider write, and the ring drops its oldest record once full.
    /// It is deliberately not the inert default — a served managed decision must record rather than
    /// refuse — while durable decision-level receipts remain outstanding (ADR 0059 / 0061).
    pub fn memory_ring() -> Result<Self, CaptureError> {
        Ok(Self::new(Box::new(MemoryCapture::new(
            CaptureMode::Memory,
            MAX_CAPTURE_RECORDS,
            MAX_CAPTURE_BYTES,
        )?)))
    }

    /// Locks the attached sink for one served release.
    ///
    /// A sink that cannot be locked cannot record, so the release is refused before the provider
    /// write rather than published for a boundary nothing observed.
    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Box<dyn CapturePort>>, DispatchError> {
        self.0.lock().map_err(|_| DispatchError::CaptureDisabled)
    }
}

impl Default for BoundaryCaptureSink {
    fn default() -> Self {
        Self::disabled()
    }
}

/// The durable prepared-dispatch ledger a served boundary persists its recorded receipts to.
///
/// One instance is shared by every session a composition opens around it, so a composition that
/// rebuilds its served session reloads the receipts the previous session committed instead of
/// starting empty and writing an already-accepted boundary a second time. Nothing here chooses a
/// store: the owner supplies the [`DispatchLedgerPort`], and the default persists nothing, so a
/// composition that attaches no store keeps the in-session ledger rather than acquiring an
/// undeclared durable surface.
#[derive(Clone, Debug)]
pub struct ServedDispatchLedger(Arc<Mutex<Box<dyn DispatchLedgerPort>>>);

impl ServedDispatchLedger {
    /// Wraps one owner-supplied durable ledger port.
    pub fn new(port: Box<dyn DispatchLedgerPort>) -> Self {
        Self(Arc::new(Mutex::new(port)))
    }

    /// The image this composition last persisted, or `None` when it never persisted one.
    pub(super) fn load(&self) -> Result<Option<DurableDispatchLedger>, DispatchLedgerError> {
        self.0
            .lock()
            .map_err(|_| DispatchLedgerError::Unavailable)?
            .load()
    }

    /// Persists one image, so a restarted composition reloads the same receipts.
    pub(super) fn save(&self, ledger: &DurableDispatchLedger) -> Result<(), DispatchLedgerError> {
        self.0
            .lock()
            .map_err(|_| DispatchLedgerError::Unavailable)?
            .save(ledger)
    }
}

impl Default for ServedDispatchLedger {
    fn default() -> Self {
        Self::new(Box::new(NoopDispatchLedgerPort))
    }
}

/// Confirms the served composition attaches a coherent observation, render and control set.
///
/// The three refusals are one rule: an enforcing owner needs the admitted control limits, admitted
/// limits need the owner that enforces them, and managed rendering needs both. They live beside the
/// ports they read, which are the only inputs they have.
pub(super) fn validate_served_owner_composition(
    observations: Option<&Arc<dyn LiveContextObservationPort>>,
    render: Option<&Arc<dyn LiveContextRenderPort>>,
    control_limits: Option<&ContextOwnerControlLimits>,
) -> Result<(), ManagementError> {
    let observed = observations.is_some();
    let admitted = control_limits.is_some();
    if observed && !admitted {
        return Err(ManagementError::capability(
            "selected_context_control_limits_required",
            "served context observations require the admitted control limits",
        ));
    }
    if admitted && !observed {
        return Err(ManagementError::capability(
            "selected_context_control_owner_unavailable",
            "admitted context control limits have no attached enforcing owner",
        ));
    }
    if render.is_some_and(|render| render.render_required()) && !(observed && admitted) {
        return Err(ManagementError::capability(
            "selected_context_render_owner_unavailable",
            "managed rendering requires the admitted observation and control owner",
        ));
    }
    Ok(())
}
