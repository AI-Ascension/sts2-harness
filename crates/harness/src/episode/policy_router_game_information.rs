// SPDX-License-Identifier: MIT

use super::*;

impl PolicyRouter {
    pub fn choose_with_game_information<
        S: DecisionSource,
        P: super::super::runner::EpisodeRuntimePort,
    >(
        source: &mut S,
        input: &DecisionInput,
        runtime: &mut P,
    ) -> Result<PolicyChoice, PolicyError> {
        input
            .observation
            .assert_actionable()
            .map_err(|_| PolicyError::InputBlocked)?;
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| PolicyError::StaleCatalog)?;
        let decision = source.decide_with_game_information(input, runtime);
        Self::route_decision(decision?, input)
    }

    pub(super) fn route_decision(
        decision: Decision,
        input: &DecisionInput,
    ) -> Result<PolicyChoice, PolicyError> {
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
            Decision::Reobserve {
                rationale,
                candidate_action_id,
                candidate_confidence,
            } => {
                // A candidate naming something the host is not offering is dropped rather than
                // carried: it could only mislead a later decision to dispatch it.
                let candidate_action_id = candidate_action_id
                    .filter(|action_id| input.legal_actions.find(action_id).is_some());
                Ok(PolicyChoice::Reobserve {
                    rationale,
                    candidate_confidence: candidate_confidence
                        .filter(|_| candidate_action_id.is_some()),
                    candidate_action_id,
                })
            }
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
