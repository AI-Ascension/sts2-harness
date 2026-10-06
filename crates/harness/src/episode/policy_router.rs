// SPDX-License-Identifier: MIT

#[path = "policy_router_choice.rs"]
mod choice;
#[path = "policy_router_exo.rs"]
mod exo_source;
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

    /// Converts the exact runtime-owned decision input to the shared managed
    /// renderer representation. This remains read-only and carries no authority.
    #[must_use]
    pub fn managed_render_input(&self) -> ManagedRenderInput {
        ManagedRenderInput {
            execution_id: self.execution_id.to_string(),
            state_id: self.observation.state_id().to_owned(),
            generation: self.observation.generation(),
            observation: self.observation.fair_play().as_value().clone(),
            legal_action_ids: self
                .legal_actions
                .actions()
                .iter()
                .map(|action| action.action_id().to_owned())
                .collect(),
            objective: self.objective.clone(),
            hard_constraints: self.hard_constraints.clone(),
            map_context: self.map_context().map(MapDecisionContext::to_wire),
        }
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

    /// Returns the exact admitted provider configuration selected for the next
    /// trusted managed-context render. Non-Exo sources remain unavailable.
    fn managed_render_config(&self) -> Option<crate::exo::ExoConfig> {
        None
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    InputBlocked,
    StaleCatalog,
    IllegalAction,
    MissingOperation,
    MalformedDecision,
    /// No request byte reached a provider: the transport never started.
    ///
    /// This is the only provider failure that proves the exchange did not happen, so callers that
    /// must not repeat a paid request may release their identity on it.
    ProviderNotStarted,
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
            Self::ProviderNotStarted => "provider transport never started; no inference was sent",
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
        ExoError::NotStarted => PolicyError::ProviderNotStarted,
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
