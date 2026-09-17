// SPDX-License-Identifier: MIT

#[path = "policy_router_choice.rs"]
mod choice;
#[path = "policy_router_managed.rs"]
mod managed_render;
pub use choice::{PolicyChoice, PolicyRouter};

use super::action_plan::ActionPlan;
use super::legal_actions::EpisodeLegalActionSet;
use super::map::MapDecisionContext;
use super::observation::EpisodeObservation;
use super::recovery::RecoveryOperation;
use crate::context_control::{ManagedRenderInput, PreparedContext};
use crate::exo::{Decision, ExoError, ExoSession};
use crate::identity::ModelExecutionId;
use crate::management::ContextRenderSource;

#[path = "policy_router_game_information.rs"]
mod game_information;

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

    /// Opt-in Runtime-v3 providers can receive bounded game-information tool
    /// feedback through the already-owned MCP runtime port.
    fn decide_with_game_information(
        &mut self,
        input: &DecisionInput,
        _runtime: &mut dyn super::runner::EpisodeRuntimePort,
    ) -> Result<Decision, PolicyError> {
        self.decide(input)
    }

    /// Routes an authored decision-profile/context binding to the provider
    /// boundary. Legacy sources inherit `decide`; bound providers can override
    /// this method to enforce or select the requested profile and context.
    fn decide_for(
        &mut self,
        _input: &DecisionInput,
        _decision_profile_ref: &str,
        _context_ref: &str,
    ) -> Result<Decision, PolicyError> {
        Err(PolicyError::ProviderUnavailable)
    }

    /// Prepares immutable managed-context bytes for the exact provider input.
    /// Sources without this capability fail closed for render-required owners.
    fn prepare_managed_context(
        &mut self,
        _input: &DecisionInput,
        _source: &ContextRenderSource,
    ) -> Result<PreparedContext, PolicyError> {
        Err(PolicyError::ProviderUnavailable)
    }

    /// Sends the exact prepared bytes associated with this decision input.
    fn decide_prepared_for(
        &mut self,
        _input: &DecisionInput,
        _decision_profile_ref: &str,
        _context_ref: &str,
        _prepared: &PreparedContext,
    ) -> Result<Decision, PolicyError> {
        Err(PolicyError::ProviderUnavailable)
    }

    /// Reports whether the selected action passed settlement verification, including recovery.
    /// False includes rejection, cancellation and unresolved failure; it never authorizes retry.
    fn action_completed(&mut self, _settled: bool) {}

    /// Originating provider execution for the most recently returned action, if retained.
    fn model_execution_id(&self) -> Option<ModelExecutionId> {
        None
    }

    fn close(&mut self) -> Result<(), PolicyError> {
        Ok(())
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
    fn close(&mut self) -> Result<(), PolicyError> {
        ExoDecisionSource::close(self).map_err(map_exo_error)
    }

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

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<Decision, PolicyError> {
        if decision_profile_ref.is_empty() || context_ref.is_empty() {
            return Err(PolicyError::InputBlocked);
        }
        self.decide(input)
    }

    fn prepare_managed_context(
        &mut self,
        input: &DecisionInput,
        source: &ContextRenderSource,
    ) -> Result<PreparedContext, PolicyError> {
        self.prepare_managed_context_impl(input, source)
    }

    fn decide_prepared_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
        prepared: &PreparedContext,
    ) -> Result<Decision, PolicyError> {
        self.decide_prepared_for_impl(input, decision_profile_ref, context_ref, prepared)
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
    SelectedContextLimit(&'static str),
    /// A per-invocation membership gate refused this dispatch before any provider exchange.
    MembershipRefused(&'static str),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::SelectedContextLimit(limit) = self {
            return write!(
                formatter,
                "managed context exceeds selected owner limit: {limit}"
            );
        }
        formatter.write_str(match self {
            Self::InputBlocked => "episode input is blocked",
            Self::StaleCatalog => "legal-action catalog is stale",
            Self::IllegalAction => "provider action is absent from the current catalog",
            Self::MissingOperation => "recovery reconciliation lacks an operation identity",
            Self::MalformedDecision => "provider decision is malformed",
            Self::ProviderUnavailable => "provider is unavailable",
            Self::ProviderMalformed => "provider request or response is malformed",
            Self::ProviderClosed => "provider session is closed",
            Self::SelectedContextLimit(_) => "managed context exceeds selected owner limit",
            Self::MembershipRefused(code) => code,
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
