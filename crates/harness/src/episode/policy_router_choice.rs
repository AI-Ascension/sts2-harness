// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyChoice {
    Action {
        action_id: String,
        rationale: String,
        confidence: Option<u8>,
    },
    Wait {
        rationale: String,
    },
    Reobserve {
        rationale: String,
        /// The option the source would have taken while declining to act, when it named one.
        ///
        /// Carried so a runner that has re-asked an unchanged state to its bound can act on what
        /// was already said. It is never dispatched on the strength of being present: the choice
        /// is still to observe again unless the runner decides otherwise.
        candidate_action_id: Option<String>,
        candidate_confidence: Option<u8>,
    },
    Recovery {
        operation: RecoveryOperation,
        rationale: String,
    },
}

pub struct PolicyRouter;

impl PolicyRouter {
    pub fn choose<S: DecisionSource>(
        source: &mut S,
        input: &DecisionInput,
    ) -> Result<PolicyChoice, PolicyError> {
        input
            .observation
            .assert_actionable()
            .map_err(|_| PolicyError::InputBlocked)?;
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| PolicyError::StaleCatalog)?;
        let decision = source.decide(input)?;
        Self::route_decision(decision, input)
    }
}
