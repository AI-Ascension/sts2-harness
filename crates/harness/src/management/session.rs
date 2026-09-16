// SPDX-License-Identifier: MIT

use super::super::service::ManagementError;
use crate::episode::{
    ActionIdentity, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeRuntimePort, RecoveryError, RecoveryPort, TransitionReceipt,
    WaitSample,
};

#[path = "session/factory.rs"]
mod factory;
pub use factory::{LiveWorkflowFactory, LiveWorkflowSessionFactory};

/// The live boundary assembled by a binary from its gateway/MCP runtime and
/// provider. No graph, transport, or game authority enters this trait.
pub trait LiveWorkflowSession: Send {
    fn launch(&mut self) -> Result<(), ManagementError>;
    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError>;
    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, ManagementError> {
        Err(ManagementError::capability(
            "live_projection_binding_unavailable",
            format!("live session does not support authored projection {projection_ref}"),
        ))
    }
    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, ManagementError>;
    fn decide(&mut self, input: &DecisionInput) -> Result<crate::Decision, ManagementError>;
    fn decide_for(
        &mut self,
        _input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<crate::Decision, ManagementError> {
        Err(ManagementError::capability(
            "live_decision_binding_unavailable",
            format!(
                "live session does not support profile {decision_profile_ref} with context {context_ref}"
            ),
        ))
    }
    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError>;
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, ManagementError>;
    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, ManagementError>;
    fn release_lease(&mut self) -> Result<(), ManagementError>;
    fn stop_episode(&mut self) -> Result<(), ManagementError>;

    fn pause(&mut self) -> Result<(), ManagementError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), ManagementError> {
        Ok(())
    }

    fn action_completed(&mut self, _settled: bool) {}

    fn model_execution_id(&self) -> Option<crate::ModelExecutionId> {
        None
    }
}

/// Generic composition over the existing episode runtime and decision ports.
pub struct EpisodeRuntimeSession<R, S> {
    runtime: R,
    source: S,
}

impl<R, S> EpisodeRuntimeSession<R, S> {
    #[must_use]
    pub fn new(runtime: R, source: S) -> Self {
        Self { runtime, source }
    }
}

impl<R, S> LiveWorkflowSession for EpisodeRuntimeSession<R, S>
where
    R: EpisodeRuntimePort + Send,
    S: DecisionSource + Send,
{
    fn launch(&mut self) -> Result<(), ManagementError> {
        self.runtime
            .launch()
            .map_err(|error| port_error("live_launch_failed", error))
    }

    fn observe(&mut self) -> Result<EpisodeObservation, ManagementError> {
        self.runtime
            .observe()
            .map_err(|error| port_error("live_observe_failed", error))
    }

    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, ManagementError> {
        if projection_ref.is_empty() {
            return Err(ManagementError::invalid(
                "live_projection_ref",
                "observe projection reference is empty",
            ));
        }
        self.runtime
            .observe_projection(projection_ref)
            .map_err(|error| port_error("live_observe_failed", error))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, ManagementError> {
        self.runtime
            .legal_actions(state_id, generation)
            .map_err(|error| port_error("live_catalog_failed", error))
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<crate::Decision, ManagementError> {
        self.source.decide(input).map_err(|error| {
            ManagementError::unavailable("provider_decision_failed", error.to_string())
        })
    }

    fn decide_for(
        &mut self,
        input: &DecisionInput,
        decision_profile_ref: &str,
        context_ref: &str,
    ) -> Result<crate::Decision, ManagementError> {
        if decision_profile_ref.is_empty() || context_ref.is_empty() {
            return Err(ManagementError::invalid(
                "live_decision_binding",
                "decision profile and context references are required",
            ));
        }
        self.source
            .decide_for(input, decision_profile_ref, context_ref)
            .map_err(|error| {
                ManagementError::unavailable("provider_decision_failed", error.to_string())
            })
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, ManagementError> {
        self.runtime
            .dispatch_action(identity, action)
            .map_err(|error| port_error("live_dispatch_failed", error))
    }

    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, ManagementError> {
        self.runtime
            .wait_for_transition(operation_id, wait_for_millis)
            .map_err(|error| {
                ManagementError::unresolved("live_transition_wait_failed", error.to_string())
            })
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, ManagementError> {
        self.runtime.reconcile(operation_id).map_err(|error| {
            ManagementError::unresolved("live_reconcile_failed", error.to_string())
        })
    }

    fn release_lease(&mut self) -> Result<(), ManagementError> {
        RecoveryPort::release_lease(&mut self.runtime)
            .map_err(|error| recovery_error("live_release_failed", error))
    }

    fn stop_episode(&mut self) -> Result<(), ManagementError> {
        RecoveryPort::stop_episode(&mut self.runtime)
            .map_err(|error| recovery_error("live_stop_failed", error))
    }

    fn action_completed(&mut self, settled: bool) {
        self.source.action_completed(settled);
    }

    fn model_execution_id(&self) -> Option<crate::ModelExecutionId> {
        self.source.model_execution_id()
    }
}

/// Bounded policy and transition settings for one live composition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveWorkflowOptions {
    pub objective: String,
    pub hard_constraints: Vec<String>,
    pub transition_wait_millis: u32,
}

impl Default for LiveWorkflowOptions {
    fn default() -> Self {
        Self {
            objective: "execute the authored workflow".to_owned(),
            hard_constraints: Vec::new(),
            transition_wait_millis: 5_000,
        }
    }
}

impl LiveWorkflowOptions {
    pub(super) fn validate(&self) -> Result<(), ManagementError> {
        let valid_constraint = |value: &String| {
            !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        };
        if !valid_constraint(&self.objective)
            || self.hard_constraints.len() > 32
            || self
                .hard_constraints
                .iter()
                .any(|value| !valid_constraint(value))
            || self.transition_wait_millis == 0
            || self.transition_wait_millis > 120_000
        {
            return Err(ManagementError::invalid(
                "live_options_invalid",
                "live workflow options are outside their bounds",
            ));
        }
        Ok(())
    }
}

fn port_error(code: &'static str, error: crate::PortError) -> ManagementError {
    if error.is_retryable() {
        ManagementError::unresolved(code, error.message())
    } else {
        ManagementError::unavailable(code, error.message())
    }
}

fn recovery_error(code: &str, error: RecoveryError) -> ManagementError {
    ManagementError::unresolved(code, error.to_string())
}
