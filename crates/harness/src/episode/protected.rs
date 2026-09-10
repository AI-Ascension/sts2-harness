// SPDX-License-Identifier: MIT

use super::idempotency::ActionIdentity;
use super::legal_actions::{EpisodeLegalAction, EpisodeLegalActionSet};
use super::observation::EpisodeObservation;
use super::runner::EpisodeRuntimePort;
use super::transition::TransitionReceipt;
use crate::error::PortError;

/// Read-only episode operations owned by the harness boundary.
///
/// The existing runtime trait remains the compatibility surface. This focused view lets future
/// workflow nodes consume observation and catalog behavior without reaching into an adapter's
/// transport or game-process implementation.
pub trait EpisodeObservationPort {
    fn observe(&mut self) -> Result<EpisodeObservation, PortError>;

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError>;
}

/// One protected mutation admission operation owned by the harness boundary.
pub trait EpisodeActionPort {
    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError>;
}

/// Lifecycle admission for an episode runtime.
pub trait EpisodeLifecyclePort {
    fn launch(&mut self) -> Result<(), PortError>;
}

/// Complete protected episode surface used by the compatibility runner and workflow execution.
///
/// The blanket implementation preserves existing `EpisodeRuntimePort` implementers while
/// exposing the narrower operation views above. No focused port can bypass the existing barrier,
/// recovery, or ordered cleanup contracts.
pub trait ProtectedEpisodePort: EpisodeRuntimePort {}

impl<T> EpisodeObservationPort for T
where
    T: EpisodeRuntimePort + ?Sized,
{
    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        EpisodeRuntimePort::observe(self)
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        EpisodeRuntimePort::legal_actions(self, state_id, generation)
    }
}

impl<T> EpisodeActionPort for T
where
    T: EpisodeRuntimePort + ?Sized,
{
    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        EpisodeRuntimePort::dispatch_action(self, identity, action)
    }
}

impl<T> EpisodeLifecyclePort for T
where
    T: EpisodeRuntimePort + ?Sized,
{
    fn launch(&mut self) -> Result<(), PortError> {
        EpisodeRuntimePort::launch(self)
    }
}

impl<T> ProtectedEpisodePort for T where T: EpisodeRuntimePort + ?Sized {}
