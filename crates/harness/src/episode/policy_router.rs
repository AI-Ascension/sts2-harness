// SPDX-License-Identifier: MIT

use super::action_plan::ActionPlan;
use super::legal_actions::EpisodeLegalActionSet;
use super::map::MapDecisionContext;
use super::observation::EpisodeObservation;
use super::recovery::RecoveryOperation;
use crate::exo::{Decision, ExoError, ExoSession};
use crate::identity::ModelExecutionId;

/// Inputs given to a provider for one current observation. The observation has already passed the
/// fair-play firewall and the action set is host-generated.
#[derive(Clone, Debug)]
pub struct DecisionInput {
    pub execution_id: ModelExecutionId,
    pub observation: EpisodeObservation,
    pub legal_actions: EpisodeLegalActionSet,
    pub objective: String,
    pub hard_constraints: Vec<String>,
}

impl DecisionInput {
    #[must_use]
    pub fn new(
        execution_id: ModelExecutionId,
        observation: EpisodeObservation,
        legal_actions: EpisodeLegalActionSet,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
    ) -> Self {
        Self {
            execution_id,
            observation,
            legal_actions,
            objective: objective.into(),
            hard_constraints,
        }
    }

    #[must_use]
    pub(crate) fn with_map_context(mut self, context: MapDecisionContext) -> Self {
        self.observation = self.observation.clone().with_map_context(context);
        self
    }

    #[must_use]
    pub(crate) fn map_context(&self) -> Option<&MapDecisionContext> {
        self.observation.map_context()
    }
}

pub trait DecisionSource {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError>;

    /// Reports whether the selected action passed settlement verification, including recovery.
    /// False includes rejection, cancellation and unresolved failure; it never authorizes retry.
    fn action_completed(&mut self, _settled: bool) {}

    /// Originating provider execution for the most recently returned action, if retained.
    fn model_execution_id(&self) -> Option<ModelExecutionId> {
        None
    }
}

/// Connects the episode policy port to the bounded Exo session.
pub struct ExoDecisionSource<T> {
    session: ExoSession<T>,
    plan: Option<ActionPlan>,
    execution_id: Option<ModelExecutionId>,
}

impl<T> ExoDecisionSource<T> {
    #[must_use]
    pub fn new(session: ExoSession<T>) -> Self {
        Self {
            session,
            plan: None,
            execution_id: None,
        }
    }

    pub fn close(&mut self) -> Result<(), ExoError>
    where
        T: crate::exo::ExoTransport,
    {
        self.plan = None;
        self.session.close()
    }
}

impl<T: crate::exo::ExoTransport> DecisionSource for ExoDecisionSource<T> {
    fn action_completed(&mut self, settled: bool) {
        if !settled {
            self.plan = None;
            return;
        }
        if let Some(plan) = &mut self.plan {
            plan.action_completed(settled);
        }
    }

    fn model_execution_id(&self) -> Option<ModelExecutionId> {
        self.execution_id
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        input
            .observation
            .assert_actionable()
            .map_err(|_| PolicyError::InputBlocked)?;
        input
            .legal_actions
            .assert_matches(input.observation.state_id(), input.observation.generation())
            .map_err(|_| PolicyError::StaleCatalog)?;
        if self
            .plan
            .as_ref()
            .is_some_and(ActionPlan::awaiting_settlement)
        {
            return Err(PolicyError::InputBlocked);
        }
        if let Some(plan) = &mut self.plan
            && let Some(decision) = plan.next(input, false)
        {
            self.execution_id = Some(plan.execution_id);
            return Ok(decision);
        }
        self.plan = None;
        self.execution_id = Some(input.execution_id);
        let legal_action_ids = input
            .legal_actions
            .actions()
            .iter()
            .map(|action| action.action_id().to_owned())
            .collect();
        let decision = if let Some(map_context) = input.map_context() {
            self.session.decide_with_map(
                input.execution_id,
                input.observation.state_id(),
                input.observation.generation(),
                input.observation.fair_play().clone(),
                legal_action_ids,
                input.objective.clone(),
                input.hard_constraints.clone(),
                map_context.clone(),
            )
        } else {
            self.session.decide(
                input.execution_id,
                input.observation.state_id(),
                input.observation.generation(),
                input.observation.fair_play().clone(),
                legal_action_ids,
                input.objective.clone(),
                input.hard_constraints.clone(),
            )
        }
        .map_err(map_exo_error)?;
        if let Decision::Plan {
            action_ids,
            rationale,
        } = decision
        {
            let mut plan = ActionPlan::new(input, &action_ids, rationale)?;
            let action = plan.next(input, true).ok_or(PolicyError::IllegalAction)?;
            self.plan = Some(plan);
            Ok(action)
        } else {
            Ok(decision)
        }
    }
}

/// A provider decision after binding to the current action set, or an explicit non-action
/// directive that the coordinator must handle.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    InputBlocked,
    StaleCatalog,
    IllegalAction,
    MissingOperation,
    MalformedDecision,
    ProviderUnavailable,
    ProviderMalformed,
    ProviderClosed,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InputBlocked => "episode input is blocked",
            Self::StaleCatalog => "legal-action catalog is stale",
            Self::IllegalAction => "provider action is absent from the current catalog",
            Self::MissingOperation => "recovery reconciliation lacks an operation identity",
            Self::MalformedDecision => "provider decision is malformed",
            Self::ProviderUnavailable => "provider is unavailable",
            Self::ProviderMalformed => "provider request or response is malformed",
            Self::ProviderClosed => "provider session is closed",
        })
    }
}

fn map_exo_error(error: ExoError) -> PolicyError {
    match error {
        ExoError::Unavailable | ExoError::Timeout => PolicyError::ProviderUnavailable,
        ExoError::Closed => PolicyError::ProviderClosed,
        ExoError::Decision(_) => PolicyError::MalformedDecision,
        ExoError::InvalidConfig
        | ExoError::InvalidRequest
        | ExoError::RequestTooLarge
        | ExoError::OversizedResponse
        | ExoError::MalformedResponse
        | ExoError::Sandbox(_) => PolicyError::ProviderMalformed,
    }
}

impl std::error::Error for PolicyError {}
