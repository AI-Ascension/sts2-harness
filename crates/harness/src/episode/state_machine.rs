// SPDX-License-Identifier: MIT

use super::observation::{EpisodeObservation, EpisodeStage};

/// Explicit coordinator phase; no phase silently chooses an action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpisodePhase {
    AwaitingObservation,
    Ready(EpisodeObservation),
    AwaitingTransition {
        operation_id: String,
        generation: u64,
    },
    Recovering {
        operation_id: Option<String>,
    },
    Complete(EpisodeStage),
    Failed,
}

/// State machine enforcing fresh observations before every dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeMachine {
    phase: EpisodePhase,
    last_generation: Option<u64>,
}

impl Default for EpisodeMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl EpisodeMachine {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: EpisodePhase::AwaitingObservation,
            last_generation: None,
        }
    }

    #[must_use]
    pub fn phase(&self) -> &EpisodePhase {
        &self.phase
    }

    pub fn observe(&mut self, observation: EpisodeObservation) -> Result<(), EpisodeMachineError> {
        if self
            .last_generation
            .is_some_and(|generation| observation.generation() < generation)
        {
            self.phase = EpisodePhase::Recovering { operation_id: None };
            return Err(EpisodeMachineError::StaleObservation);
        }
        self.last_generation = Some(observation.generation());
        if observation.stage() == EpisodeStage::Unknown {
            self.phase = EpisodePhase::Recovering { operation_id: None };
            return Err(EpisodeMachineError::UnknownState);
        }
        if observation.stage().is_terminal() {
            self.phase = EpisodePhase::Complete(observation.stage());
        } else {
            self.phase = EpisodePhase::Ready(observation);
        }
        Ok(())
    }

    pub fn begin_dispatch(
        &mut self,
        operation_id: impl Into<String>,
    ) -> Result<(), EpisodeMachineError> {
        let operation_id = operation_id.into();
        if operation_id.is_empty() {
            return Err(EpisodeMachineError::InvalidOperation);
        }
        let EpisodePhase::Ready(observation) = &self.phase else {
            return Err(EpisodeMachineError::NotReady);
        };
        observation
            .assert_actionable()
            .map_err(|_| EpisodeMachineError::InputBlocked)?;
        self.phase = EpisodePhase::AwaitingTransition {
            operation_id,
            generation: observation.generation(),
        };
        Ok(())
    }

    pub fn require_recovery(&mut self, operation_id: Option<String>) {
        self.phase = EpisodePhase::Recovering { operation_id };
    }

    pub fn settle(&mut self, observation: EpisodeObservation) -> Result<(), EpisodeMachineError> {
        let EpisodePhase::AwaitingTransition {
            operation_id,
            generation,
        } = &self.phase
        else {
            return Err(EpisodeMachineError::NotAwaitingTransition);
        };
        if observation.generation() <= *generation {
            self.phase = EpisodePhase::Recovering {
                operation_id: Some(operation_id.clone()),
            };
            return Err(EpisodeMachineError::StaleTransition);
        }
        if observation.stage() == EpisodeStage::Unknown
            || observation.stage() == EpisodeStage::Recovery
        {
            self.phase = EpisodePhase::Recovering {
                operation_id: Some(operation_id.clone()),
            };
            return Err(EpisodeMachineError::UnknownState);
        }
        self.last_generation = Some(observation.generation());
        if observation.stage().is_terminal() {
            self.phase = EpisodePhase::Complete(observation.stage());
        } else {
            self.phase = EpisodePhase::Ready(observation);
        }
        Ok(())
    }

    pub fn fail(&mut self) {
        self.phase = EpisodePhase::Failed;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpisodeMachineError {
    UnknownState,
    StaleObservation,
    InvalidOperation,
    NotReady,
    InputBlocked,
    NotAwaitingTransition,
    StaleTransition,
}

impl std::fmt::Display for EpisodeMachineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnknownState => "episode entered unknown state",
            Self::StaleObservation => {
                "episode observation is older than the last accepted generation"
            }
            Self::InvalidOperation => "operation identity is invalid",
            Self::NotReady => "episode is not ready for dispatch",
            Self::InputBlocked => "episode input is blocked",
            Self::NotAwaitingTransition => "episode is not awaiting a transition",
            Self::StaleTransition => "transition did not produce a fresh generation",
        })
    }
}

impl std::error::Error for EpisodeMachineError {}
