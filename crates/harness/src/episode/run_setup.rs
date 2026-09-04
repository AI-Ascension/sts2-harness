// SPDX-License-Identifier: MIT

use super::legal_actions::EpisodeLegalActionSet;
use super::observation::{EpisodeObservation, EpisodeStage};
use super::policy_router::{
    DecisionInput, DecisionSource, PolicyChoice, PolicyError, PolicyRouter,
};
use crate::identity::ModelExecutionId;

pub trait SetupPort {
    fn start_run(&mut self) -> Result<(), PolicyError>;
}

/// Routes setup/character decisions through the configured provider.
pub struct RunSetupCoordinator;

impl RunSetupCoordinator {
    pub fn choose<S: DecisionSource>(
        &self,
        source: &mut S,
        execution_id: ModelExecutionId,
        observation: EpisodeObservation,
        legal_actions: EpisodeLegalActionSet,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
    ) -> Result<PolicyChoice, PolicyError> {
        if observation.stage() != EpisodeStage::Setup {
            return Err(PolicyError::InputBlocked);
        }
        PolicyRouter::choose(
            source,
            &DecisionInput::new(
                execution_id,
                observation,
                legal_actions,
                objective,
                hard_constraints,
            ),
        )
    }
}
