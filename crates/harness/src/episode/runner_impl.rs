// SPDX-License-Identifier: MIT

use super::super::idempotency::{ActionIdentity, ActionLedger, Admission};
use super::super::legal_actions::{EpisodeLegalAction, EpisodeLegalActionSet};
use super::super::observation::EpisodeObservation;
use super::super::policy_router::{DecisionInput, DecisionSource, PolicyChoice, PolicyRouter};
use super::super::postconditions::verify_settlement;
use super::super::recovery::{RecoveryOperation, RecoveryResult};
use super::super::state_machine::{EpisodeMachine, EpisodeMachineError};
use super::super::transition::{DispatchStatus, TransitionReceipt};
use super::runner_recovery::{accept_observation, report};
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};
use crate::identity::ModelExecutionId;

#[derive(Default)]
struct RunCounters {
    transitions: u32,
    recoveries: u32,
}

enum ObservationStep {
    Ready(EpisodeObservation),
    Retry,
    Complete(EpisodeObservation),
}

struct ActionExecution {
    operation_id: String,
    identity: ActionIdentity,
    action: EpisodeLegalAction,
}

impl EpisodeRunner {
    pub(super) fn run_inner<P: EpisodeRuntimePort, S: DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
    ) -> Result<EpisodeRunReport, EpisodeRunnerError> {
        let mut machine = EpisodeMachine::new();
        let mut ledger = ActionLedger::new(self.config.max_steps as usize)
            .map_err(EpisodeRunnerError::Ledger)?;
        let mut counters = RunCounters::default();
        for step in 0..self.config.max_steps {
            if let Some(report) =
                self.run_step(port, source, &mut machine, &mut ledger, step, &mut counters)?
            {
                return Ok(report);
            }
        }
        Err(EpisodeRunnerError::StepLimitExceeded)
    }

    fn run_step<P: EpisodeRuntimePort, S: DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
        machine: &mut EpisodeMachine,
        ledger: &mut ActionLedger,
        step: u32,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        let observation = match self.prepare_observation(port, machine, counters, step)? {
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
        _step: u32,
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

    fn handle_action<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        ledger: &mut ActionLedger,
        observation: &EpisodeObservation,
        legal_actions: &EpisodeLegalActionSet,
        action_id: &str,
        step_number: u32,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        let execution = prepare_action(
            machine,
            ledger,
            observation,
            legal_actions,
            action_id,
            step_number,
        )?;
        let receipt = match port.dispatch_action(&execution.identity, &execution.action) {
            Ok(receipt) => receipt,
            Err(_error) => {
                return self.recover_action(
                    port,
                    machine,
                    observation,
                    &execution,
                    step_number,
                    counters,
                );
            }
        };
        self.handle_receipt(
            port,
            machine,
            observation,
            &execution,
            receipt,
            step_number,
            counters,
        )
    }

    fn handle_receipt<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        observation: &EpisodeObservation,
        execution: &ActionExecution,
        receipt: TransitionReceipt,
        step_number: u32,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        match receipt.status() {
            DispatchStatus::Accepted | DispatchStatus::Settled => {
                match self.settle_dispatch(
                    port,
                    machine,
                    observation,
                    &execution.operation_id,
                    &execution.action,
                    &receipt,
                ) {
                    Ok(after) => complete_transition(after, step_number, counters),
                    Err(_) => self.recover_action(
                        port,
                        machine,
                        observation,
                        execution,
                        step_number,
                        counters,
                    ),
                }
            }
            DispatchStatus::Unknown => {
                self.recover_action(port, machine, observation, execution, step_number, counters)
            }
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                machine.require_recovery(Some(execution.operation_id.clone()));
                self.reobserve(port, machine)?;
                counters.recoveries += 1;
                Ok(None)
            }
        }
    }

    fn recover_action<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        observation: &EpisodeObservation,
        execution: &ActionExecution,
        step_number: u32,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        machine.require_recovery(Some(execution.operation_id.clone()));
        let after = self.reconcile_uncertain(
            port,
            machine,
            observation,
            &execution.operation_id,
            &execution.action,
        )?;
        counters.recoveries += 1;
        Ok(after.and_then(|after| {
            counters.transitions += 1;
            after.stage().is_terminal().then(|| {
                report(
                    after,
                    step_number,
                    counters.transitions,
                    counters.recoveries,
                )
            })
        }))
    }

    fn handle_wait<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        observation: &EpisodeObservation,
        step_number: u32,
        counters: &mut RunCounters,
    ) -> Result<(), EpisodeRunnerError> {
        let operation_id = format!("episode-wait-{}-{}", observation.generation(), step_number);
        match self
            .config
            .barrier
            .await_transition_sample(port, &operation_id, observation)
        {
            Ok(sample) => {
                let after = sample
                    .observation()
                    .cloned()
                    .ok_or(EpisodeRunnerError::MissingObservation)?;
                accept_observation(machine, after)?;
            }
            Err(_) => {
                self.reobserve(port, machine)?;
                counters.recoveries += 1;
            }
        }
        Ok(())
    }

    fn handle_recovery<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        observation: &EpisodeObservation,
        operation: RecoveryOperation,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        let result = self
            .config
            .recovery
            .recover(port, operation)
            .map_err(EpisodeRunnerError::Recovery)?;
        match result {
            RecoveryResult::Observation(after) => {
                accept_observation(machine, after)?;
                counters.recoveries += 1;
                Ok(None)
            }
            RecoveryResult::Receipt(receipt) => {
                finish_recovery_receipt(machine, observation, receipt, counters)
            }
            RecoveryResult::Released | RecoveryResult::Stopped => {
                machine.fail();
                Err(EpisodeRunnerError::StoppedByRecovery)
            }
        }
    }
}

fn complete_transition(
    after: EpisodeObservation,
    step_number: u32,
    counters: &mut RunCounters,
) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
    counters.transitions += 1;
    Ok(after.stage().is_terminal().then(|| {
        report(
            after,
            step_number,
            counters.transitions,
            counters.recoveries,
        )
    }))
}

fn finish_recovery_receipt(
    machine: &mut EpisodeMachine,
    observation: &EpisodeObservation,
    receipt: TransitionReceipt,
    counters: &mut RunCounters,
) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
    if receipt.status() != DispatchStatus::Settled {
        return Err(EpisodeRunnerError::UncertainMutation);
    }
    let after = receipt
        .after()
        .cloned()
        .ok_or(EpisodeRunnerError::MissingObservation)?;
    verify_settlement(observation, &receipt).map_err(EpisodeRunnerError::Postcondition)?;
    accept_observation(machine, after)?;
    counters.recoveries += 1;
    Ok(None)
}

fn prepare_action(
    machine: &mut EpisodeMachine,
    ledger: &mut ActionLedger,
    observation: &EpisodeObservation,
    legal_actions: &EpisodeLegalActionSet,
    action_id: &str,
    step_number: u32,
) -> Result<ActionExecution, EpisodeRunnerError> {
    let action = legal_actions
        .find(action_id)
        .cloned()
        .ok_or(EpisodeRunnerError::ActionNotCurrent)?;
    let operation_id = format!(
        "episode-action-{}-{}",
        observation.generation(),
        step_number
    );
    let identity = ActionIdentity::new(
        operation_id.clone(),
        observation.state_id().to_owned(),
        observation.generation(),
        action.action_id().to_owned(),
    )
    .map_err(|_| EpisodeRunnerError::InvalidIdentity)?;
    match ledger
        .admit(identity.clone())
        .map_err(EpisodeRunnerError::Ledger)?
    {
        Admission::New => {}
        Admission::Duplicate => return Err(EpisodeRunnerError::DuplicateOperation),
        Admission::Conflict => return Err(EpisodeRunnerError::ConflictingOperation),
    }
    machine
        .begin_dispatch(operation_id.clone())
        .map_err(EpisodeRunnerError::Machine)?;
    Ok(ActionExecution {
        operation_id,
        identity,
        action,
    })
}
