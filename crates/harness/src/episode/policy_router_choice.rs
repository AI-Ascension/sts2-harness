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
        match decision {
            Decision::Plan { .. } => Err(PolicyError::MalformedDecision),
            Decision::Action {
                action_id,
                rationale,
                confidence,
            } => {
                if input.legal_actions.find(&action_id).is_none() {
                    return Err(PolicyError::IllegalAction);
                }
                Ok(PolicyChoice::Action {
                    action_id,
                    rationale,
                    confidence,
                })
            }
            Decision::Wait { rationale } => Ok(PolicyChoice::Wait { rationale }),
            Decision::Reobserve { rationale } => Ok(PolicyChoice::Reobserve { rationale }),
            Decision::Recovery {
                kind,
                operation_id,
                rationale,
            } => {
                let operation = match kind.as_str() {
                    "reobserve" => RecoveryOperation::Reobserve,
                    "reconcile" => RecoveryOperation::Reconcile {
                        operation_id: operation_id.ok_or(PolicyError::MissingOperation)?,
                    },
                    "release_lease" => RecoveryOperation::ReleaseLease,
                    "stop_episode" => RecoveryOperation::StopEpisode,
                    _ => return Err(PolicyError::MalformedDecision),
                };
                Ok(PolicyChoice::Recovery {
                    operation,
                    rationale,
                })
            }
        }
    }
}
