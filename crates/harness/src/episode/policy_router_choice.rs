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
