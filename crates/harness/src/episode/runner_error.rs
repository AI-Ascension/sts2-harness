// SPDX-License-Identifier: MIT

use super::super::idempotency::IdempotencyError;
use super::super::legal_actions::ActionSetError;
use super::super::observation::ObservationError;
use super::super::policy_router::PolicyError;
use super::super::postconditions::PostconditionError;
use super::super::recovery::RecoveryError;
use super::super::shutdown::ShutdownError;
use super::super::stability_barrier::BarrierError;
use super::super::state_machine::EpisodeMachineError;
use crate::error::PortError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpisodeRunnerError {
    InvalidConfiguration,
    InvalidIdentity,
    Launch(PortError),
    Observe(PortError),
    LegalActions(PortError),
    Dispatch(PortError),
    Barrier(BarrierError),
    Recovery(RecoveryError),
    Shutdown(ShutdownError),
    Machine(EpisodeMachineError),
    Observation(ObservationError),
    ActionSet(ActionSetError),
    Policy(PolicyError),
    Ledger(IdempotencyError),
    Postcondition(PostconditionError),
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
            Self::Barrier(_) => "episode transition barrier failed",
            Self::Recovery(_) => "episode recovery failed",
            Self::Shutdown(_) => "episode cleanup failed",
            Self::Machine(_) => "episode state machine rejected a transition",
            Self::Observation(_) => "episode observation is invalid",
            Self::ActionSet(_) => "episode legal-action set is invalid",
            Self::Policy(_) => "episode policy decision was rejected",
            Self::Ledger(_) => "episode action ledger failed",
            Self::Postcondition(_) => "episode postcondition was not independently verified",
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
