// SPDX-License-Identifier: MIT

use super::legal_actions::EpisodeLegalActionSet;
use super::observation::{EpisodeObservation, EpisodeStage};
use super::policy_router::{
    DecisionInput, DecisionSource, PolicyChoice, PolicyError, PolicyRouter,
};
use crate::identity::ModelExecutionId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoncombatStage {
    Map,
    Reward,
    Shop,
    Event,
    Rest,
    Selection,
}

impl NoncombatStage {
    const fn matches(self, stage: EpisodeStage) -> bool {
        matches!(
            (self, stage),
            (Self::Map, EpisodeStage::Map)
                | (Self::Reward, EpisodeStage::Reward)
                | (Self::Shop, EpisodeStage::Shop)
                | (Self::Event, EpisodeStage::Event)
                | (Self::Rest, EpisodeStage::Rest)
                | (Self::Selection, EpisodeStage::Selection)
        )
    }
}

/// Routes all non-combat gameplay choices to the provider.
pub struct NoncombatCoordinator;

impl NoncombatCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub fn choose<S: DecisionSource>(
        &self,
        source: &mut S,
        stage: NoncombatStage,
        execution_id: ModelExecutionId,
        observation: EpisodeObservation,
        legal_actions: EpisodeLegalActionSet,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
    ) -> Result<PolicyChoice, PolicyError> {
        if !stage.matches(observation.stage()) {
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
