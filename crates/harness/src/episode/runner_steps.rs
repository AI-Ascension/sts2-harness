// SPDX-License-Identifier: MIT

use super::super::idempotency::{ActionIdentity, ActionLedger};
use super::super::legal_actions::{EpisodeLegalAction, EpisodeLegalActionSet};
use super::super::observation::EpisodeObservation;
use super::super::policy_router::{DecisionInput, DecisionSource, PolicyChoice, PolicyRouter};
use super::super::recovery::RecoveryError;
use super::super::state_machine::{EpisodeMachine, EpisodeMachineError};
use super::runner_actions::ActionRequest;
use super::runner_recovery::{accept_observation, report};
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};
use crate::identity::ModelExecutionId;

#[derive(Default)]
pub(super) struct RunCounters {
    pub(super) transitions: u32,
    pub(super) recoveries: u32,
    catalog_refreshes: u8,
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
        let mut observation = match self.prepare_observation(port, machine, counters)? {
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
        let (legal_actions, choice) = loop {
            match self.choose_policy(port, source, &observation, step) {
                Ok(result) => break result,
                Err(EpisodeRunnerError::LegalActions(error))
                    if error.code() == "catalog_reobserve" && error.is_retryable() =>
                {
                    if counters.catalog_refreshes >= 3 {
                        return Err(EpisodeRunnerError::Recovery(RecoveryError::Exhausted));
                    }
                    counters.catalog_refreshes += 1;
                    loop {
                        match catalog_reobserve_once(port, machine) {
                            Ok(fresh) => {
                                counters.recoveries += 1;
                                match self.route_observation(port, machine, fresh)? {
                                    ObservationStep::Ready(next) => observation = next,
                                    ObservationStep::Retry => return Ok(None),
                                    ObservationStep::Complete(terminal) => {
                                        return Ok(Some(report(
                                            terminal,
                                            step,
                                            counters.transitions,
                                            counters.recoveries,
                                        )));
                                    }
                                }
                                break;
                            }
                            Err(EpisodeRunnerError::Recovery(RecoveryError::PortFailure))
                                if counters.catalog_refreshes < 3 =>
                            {
                                counters.catalog_refreshes += 1;
                                counters.recoveries += 1;
                            }
                            Err(EpisodeRunnerError::Recovery(RecoveryError::PortFailure)) => {
                                return Err(EpisodeRunnerError::Recovery(RecoveryError::Exhausted));
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                Err(error) => return Err(error),
            }
        };
        counters.catalog_refreshes = 0;
        match choice {
            PolicyChoice::Action { action_id, .. } => {
                let transitions_before = counters.transitions;
                let result = self.handle_action(
                    port,
                    machine,
                    ledger,
                    ActionRequest {
                        observation: &observation,
                        legal_actions: &legal_actions,
                        action_id: &action_id,
                        step_number: step + 1,
                    },
                    counters,
                );
                source
                    .action_completed(result.is_ok() && counters.transitions > transitions_before);
                result
            }
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
            Ok(()) => self.route_observation(port, machine, observation),
            Err(EpisodeMachineError::UnknownState | EpisodeMachineError::StaleObservation) => {
                let fresh = self.reobserve(port, machine)?;
                counters.recoveries += 1;
                self.route_observation(port, machine, fresh)
            }
            Err(error) => Err(EpisodeRunnerError::Machine(error)),
        }
    }

    fn route_observation<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        observation: EpisodeObservation,
    ) -> Result<ObservationStep, EpisodeRunnerError> {
        if observation.stage().is_terminal() {
            return Ok(ObservationStep::Complete(observation));
        }
        if observation.assert_actionable().is_err() {
            let operation_id = format!("episode-idle-{}", observation.generation());
            let after = self
                .config
                .barrier
                .await_transition(port, &operation_id, &observation)
                .map_err(EpisodeRunnerError::Barrier)?;
            machine
                .observe(after.clone())
                .map_err(EpisodeRunnerError::Machine)?;
            if after.stage().is_terminal() {
                return Ok(ObservationStep::Complete(after));
            }
            if after.assert_actionable().is_err() {
                return Ok(ObservationStep::Retry);
            }
            return Ok(ObservationStep::Ready(after));
        }
        Ok(ObservationStep::Ready(observation))
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

fn catalog_reobserve_once<P: EpisodeRuntimePort>(
    port: &mut P,
    machine: &mut EpisodeMachine,
) -> Result<EpisodeObservation, EpisodeRunnerError> {
    let observation = port.reobserve().map_err(EpisodeRunnerError::Recovery)?;
    accept_observation(machine, observation.clone())?;
    Ok(observation)
}
