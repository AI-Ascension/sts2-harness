// SPDX-License-Identifier: MIT

use super::super::idempotency::IdempotencyError;
use super::super::legal_actions::ActionSetError;
use super::super::observation::ObservationError;
use super::super::policy_router::PolicyError;
use super::super::postconditions::PostconditionError;
use super::super::recovery::RecoveryError;
use super::super::shutdown::{EpisodeCleanupReport, ShutdownError};
use super::super::stability_barrier::BarrierError;
use super::super::state_machine::EpisodeMachineError;
use crate::error::PortError;

/// Primary execution failure plus cleanup evidence retained by `EpisodeRunner::run`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeRunFailure {
    primary: Box<EpisodeRunnerError>,
    cleanup: EpisodeCleanupReport,
    pending_operation_id: Option<String>,
}

impl EpisodeRunFailure {
    pub(super) fn new(
        primary: EpisodeRunnerError,
        cleanup: EpisodeCleanupReport,
        pending_operation_id: Option<String>,
    ) -> Self {
        Self {
            primary: Box::new(primary),
            cleanup,
            pending_operation_id,
        }
    }

    #[must_use]
    pub fn primary(&self) -> &EpisodeRunnerError {
        &self.primary
    }

    #[must_use]
    pub fn cleanup(&self) -> &EpisodeCleanupReport {
        &self.cleanup
    }

    #[must_use]
    pub fn pending_operation_id(&self) -> Option<&str> {
        self.pending_operation_id.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpisodeRunnerError {
    InvalidConfiguration,
    InvalidIdentity,
    Launch(PortError),
    Observe(PortError),
    LegalActions(PortError),
    Dispatch(PortError),
    DispatchRecovery {
        operation_id: String,
        dispatch: PortError,
        recovery: Box<EpisodeRunnerError>,
    },
    Barrier(BarrierError),
    Recovery(RecoveryError),
    Shutdown(ShutdownError),
    Machine(EpisodeMachineError),
    Observation(ObservationError),
    ActionSet(ActionSetError),
    Policy(PolicyError),
    Ledger(IdempotencyError),
    Postcondition(PostconditionError),
    Cleanup(EpisodeRunFailure),
    ActionNotCurrent,
    DuplicateOperation,
    ConflictingOperation,
    MissingObservation,
    MissingEffectWitness,
    RecoveryRequired,
    UnexpectedRecoveryResult,
    UncertainMutation,
    StoppedByRecovery,
    StepLimitExceeded,
}

impl std::fmt::Display for EpisodeRunnerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidConfiguration => "episode runner configuration is invalid",
            Self::InvalidIdentity => "episode runner identity allocation failed",
            Self::Launch(_) => "episode launch failed",
            Self::Observe(_) => "episode observation failed",
            Self::LegalActions(_) => "episode legal-action request failed",
            Self::Dispatch(_) => "episode action dispatch failed",
            Self::DispatchRecovery { operation_id, .. } => {
                return write!(
                    formatter,
                    "episode operation {operation_id} failed during dispatch and recovery"
                );
            }
            Self::Barrier(error) => {
                return write!(formatter, "episode transition barrier failed: {error}");
            }
            Self::Recovery(_) => "episode recovery failed",
            Self::Shutdown(_) => "episode cleanup failed",
            Self::Machine(_) => "episode state machine rejected a transition",
            Self::Observation(_) => "episode observation is invalid",
            Self::ActionSet(_) => "episode legal-action set is invalid",
            Self::Policy(error) => {
                return write!(formatter, "episode policy decision was rejected: {error}");
            }
            Self::Ledger(_) => "episode action ledger failed",
            Self::Postcondition(_) => "episode postcondition was not independently verified",
            Self::Cleanup(failure) => {
                return write!(
                    formatter,
                    "episode failed: {}; cleanup: {}",
                    failure.primary(),
                    failure.cleanup()
                );
            }
            Self::ActionNotCurrent => "provider action is not in the current host catalog",
            Self::DuplicateOperation => "episode operation was already admitted",
            Self::ConflictingOperation => "episode operation identity conflicts",
            Self::MissingObservation => "episode transition omitted an observation",
            Self::MissingEffectWitness => "episode transition omitted an effect witness",
            Self::RecoveryRequired => "episode requires recovery before policy can continue",
            Self::UnexpectedRecoveryResult => "recovery returned an unexpected result",
            Self::UncertainMutation => "mutation outcome is uncertain; episode is fail-closed",
            Self::StoppedByRecovery => "episode stopped by explicit recovery",
            Self::StepLimitExceeded => "episode exceeded its bounded step budget",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for EpisodeRunnerError {}
