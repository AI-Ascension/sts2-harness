// SPDX-License-Identifier: MIT

mod coop;
mod idempotency;
mod legal_actions;
mod noncombat;
mod observation;
mod policy_router;
mod postconditions;
mod recovery;
mod run_setup;
mod runner;
mod shutdown;
mod stability_barrier;
mod state_machine;
mod transition;

pub use coop::{CoopCoordinator, CoopError, CoopPeerRole, CoopSyncStatus};
pub use idempotency::{
    ActionIdentity, ActionLedger, Admission as ActionAdmission, IdempotencyError,
};
pub use legal_actions::{ActionKind, ActionSetError, EpisodeLegalAction, EpisodeLegalActionSet};
pub use noncombat::{NoncombatCoordinator, NoncombatStage};
pub use observation::{EpisodeObservation, EpisodeStage, ObservationError};
pub use policy_router::{
    DecisionInput, DecisionSource, ExoDecisionSource, PolicyChoice, PolicyError, PolicyRouter,
};
pub use postconditions::{PostconditionError, VerifiedTransition, verify_settlement};
pub use recovery::{
    RecoveryController, RecoveryError, RecoveryOperation, RecoveryPort, RecoveryResult,
};
pub use run_setup::{RunSetupCoordinator, SetupPort};
pub use runner::{
    EpisodeRunReport, EpisodeRunner, EpisodeRunnerConfig, EpisodeRunnerError, EpisodeRuntimePort,
};
pub use shutdown::{EpisodeShutdown, ShutdownError, ShutdownPort};
pub use stability_barrier::{BarrierError, BarrierPort, StabilityBarrier, WaitOutcome, WaitSample};
pub use state_machine::{EpisodeMachine, EpisodeMachineError, EpisodePhase};
pub use transition::{DispatchStatus, TransitionReceipt};

/// Coordinator-local error for an invalid episode request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpisodeError {
    InvalidIdentity,
    InvalidGeneration,
    UnknownState,
    InputBlocked,
    ProviderUnavailable,
    ProviderMalformed,
    StaleObservation,
    ActionNotCurrent,
    RecoveryRequired,
    TransitionUnverified,
}

impl std::fmt::Display for EpisodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "episode identity is invalid",
            Self::InvalidGeneration => "episode generation is invalid",
            Self::UnknownState => "episode state is unknown",
            Self::InputBlocked => "episode input is blocked",
            Self::ProviderUnavailable => "provider is unavailable; episode is fail-closed",
            Self::ProviderMalformed => "provider decision is malformed or illegal",
            Self::StaleObservation => "episode observation is stale",
            Self::ActionNotCurrent => "action is not in the current host catalog",
            Self::RecoveryRequired => "episode requires explicit recovery",
            Self::TransitionUnverified => "action transition lacks independent verification",
        })
    }
}

impl std::error::Error for EpisodeError {}
