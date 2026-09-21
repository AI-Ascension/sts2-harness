// SPDX-License-Identifier: MIT

use super::*;
use crate::context_capture::{
    CaptureError, CaptureMode, CapturePort, MAX_CAPTURE_BYTES, MAX_CAPTURE_RECORDS, MemoryCapture,
    NoopCapture,
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

/// Why the served boundary capture sink could not be locked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureSinkUnavailable;

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
    pub(crate) fn lock(
        &self,
    ) -> Result<MutexGuard<'_, Box<dyn CapturePort>>, CaptureSinkUnavailable> {
        self.0.lock().map_err(|_| CaptureSinkUnavailable)
    }
}

impl Default for BoundaryCaptureSink {
    fn default() -> Self {
        Self::disabled()
    }
}
