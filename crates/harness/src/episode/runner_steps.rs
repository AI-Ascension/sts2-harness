// SPDX-License-Identifier: MIT

use super::super::idempotency::{ActionIdentity, ActionLedger};
use super::super::legal_actions::{EpisodeLegalAction, EpisodeLegalActionSet};
use super::super::observation::EpisodeObservation;
use super::super::policy_router::{DecisionInput, DecisionSource, PolicyChoice, PolicyRouter};
use super::super::state_machine::{EpisodeMachine, EpisodeMachineError};
use super::runner_recovery::report;
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};
use crate::identity::ModelExecutionId;

#[derive(Default)]
pub(super) struct RunCounters {
    pub(super) transitions: u32,
    pub(super) recoveries: u32,
}

pub(super) enum ObservationStep {
    Ready(EpisodeObservation),
    Retry,
    Complete(EpisodeObservation),
}

pub(super) struct ActionExecution {
    pub(super) operation_id: String,
    pub(super) identity: ActionIdentity,
    pub(super) action: EpisodeLegalAction,
}

impl EpisodeRunner {
    pub(super) fn run_step<P: EpisodeRuntimePort, S: DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
        machine: &mut EpisodeMachine,
        ledger: &mut ActionLedger,
        step: u32,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        let observation = match self.prepare_observation(port, machine, counters)? {
            ObservationStep::Retry => return Ok(None),
            ObservationStep::Complete(observation) => {
                return Ok(Some(report(
                    observation,
                    step,
                    counters.transitions,
                    counters.recoveries,
                )));
            }
            ObservationStep::Ready(observation) => observation,
        };
        let (legal_actions, choice) = self.choose_policy(port, source, &observation, step)?;
        match choice {
            PolicyChoice::Action { action_id, .. } => self.handle_action(
                port,
                machine,
                ledger,
                &observation,
                &legal_actions,
                &action_id,
                step + 1,
                counters,
            ),
            PolicyChoice::Wait { .. } => {
                self.handle_wait(port, machine, &observation, step + 1, counters)?;
                Ok(None)
            }
            PolicyChoice::Reobserve { .. } => {
                self.reobserve(port, machine)?;
                counters.recoveries += 1;
                Ok(None)
            }
            PolicyChoice::Recovery { operation, .. } => {
                self.handle_recovery(port, machine, &observation, operation, counters)
            }
        }
    }

    fn prepare_observation<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        counters: &mut RunCounters,
    ) -> Result<ObservationStep, EpisodeRunnerError> {
        let observation = port.observe().map_err(EpisodeRunnerError::Observe)?;
        match machine.observe(observation.clone()) {
            Ok(()) if observation.stage().is_terminal() => {
                Ok(ObservationStep::Complete(observation))
            }
            Ok(()) => Ok(ObservationStep::Ready(observation)),
            Err(EpisodeMachineError::UnknownState | EpisodeMachineError::StaleObservation) => {
                let fresh = self.reobserve(port, machine)?;
                counters.recoveries += 1;
                if fresh.stage().is_terminal() {
                    Ok(ObservationStep::Complete(fresh))
                } else {
                    Ok(ObservationStep::Retry)
                }
            }
            Err(error) => Err(EpisodeRunnerError::Machine(error)),
        }
    }

    fn choose_policy<P: EpisodeRuntimePort, S: DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
        observation: &EpisodeObservation,
        step: u32,
    ) -> Result<(EpisodeLegalActionSet, PolicyChoice), EpisodeRunnerError> {
        let legal_actions = port
            .legal_actions(observation.state_id(), observation.generation())
            .map_err(EpisodeRunnerError::LegalActions)?;
        legal_actions
            .assert_matches(observation.state_id(), observation.generation())
            .map_err(EpisodeRunnerError::ActionSet)?;
        let execution_id = ModelExecutionId::new(u64::from(step + 1))
            .ok_or(EpisodeRunnerError::InvalidIdentity)?;
        let input = DecisionInput::new(
            execution_id,
            observation.clone(),
            legal_actions.clone(),
            self.config.objective.clone(),
            self.config.hard_constraints.clone(),
        );
        let choice = PolicyRouter::choose(source, &input).map_err(EpisodeRunnerError::Policy)?;
        Ok((legal_actions, choice))
    }
}
