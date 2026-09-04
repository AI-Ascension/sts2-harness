// SPDX-License-Identifier: MIT

use super::super::idempotency::{ActionIdentity, ActionLedger, Admission};
use super::super::legal_actions::EpisodeLegalAction;
use super::super::observation::EpisodeObservation;
use super::super::policy_router::{DecisionInput, DecisionSource, PolicyChoice, PolicyRouter};
use super::super::postconditions::verify_settlement;
use super::super::recovery::{RecoveryOperation, RecoveryResult};
use super::super::state_machine::{EpisodeMachine, EpisodeMachineError};
use super::super::transition::{DispatchStatus, TransitionReceipt};
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};
use crate::identity::ModelExecutionId;

impl EpisodeRunner {
    pub(super) fn run_inner<P: EpisodeRuntimePort, S: DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
    ) -> Result<EpisodeRunReport, EpisodeRunnerError> {
        let mut machine = EpisodeMachine::new();
        let mut ledger = ActionLedger::new(self.config.max_steps as usize)
            .map_err(EpisodeRunnerError::Ledger)?;
        let mut transitions = 0_u32;
        let mut recoveries = 0_u32;

        for step in 0..self.config.max_steps {
            let step_number = step + 1;
            let observation = port.observe().map_err(EpisodeRunnerError::Observe)?;
            if let Err(error) = machine.observe(observation.clone()) {
                match error {
                    EpisodeMachineError::UnknownState | EpisodeMachineError::StaleObservation => {
                        let fresh = self.reobserve(port, &mut machine)?;
                        recoveries += 1;
                        if fresh.stage().is_terminal() {
                            return Ok(report(fresh, step, transitions, recoveries));
                        }
                        continue;
                    }
                    other => return Err(EpisodeRunnerError::Machine(other)),
                }
            }
            if observation.stage().is_terminal() {
                return Ok(report(observation, step, transitions, recoveries));
            }

            let legal_actions = port
                .legal_actions(observation.state_id(), observation.generation())
                .map_err(EpisodeRunnerError::LegalActions)?;
            legal_actions
                .assert_matches(observation.state_id(), observation.generation())
                .map_err(EpisodeRunnerError::ActionSet)?;
            let execution_id = ModelExecutionId::new(u64::from(step_number))
                .ok_or(EpisodeRunnerError::InvalidIdentity)?;
            let input = DecisionInput::new(
                execution_id,
                observation.clone(),
                legal_actions.clone(),
                self.config.objective.clone(),
                self.config.hard_constraints.clone(),
            );
            let choice = PolicyRouter::choose(source, &input).map_err(EpisodeRunnerError::Policy)?;

            match choice {
                PolicyChoice::Action { action_id, .. } => {
                    let action = legal_actions
                        .find(&action_id)
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
                        Admission::Duplicate => {
                            return Err(EpisodeRunnerError::DuplicateOperation);
                        }
                        Admission::Conflict => {
                            return Err(EpisodeRunnerError::ConflictingOperation);
                        }
                    }
                    machine
                        .begin_dispatch(operation_id.clone())
                        .map_err(EpisodeRunnerError::Machine)?;
                    let receipt = match port.dispatch_action(&identity, &action) {
                        Ok(receipt) => receipt,
                        Err(_error) => {
                            machine.require_recovery(Some(operation_id.clone()));
                            let after = self.reconcile_uncertain(
                                port,
                                &mut machine,
                                &observation,
                                &operation_id,
                            )?;
                            recoveries += 1;
                            if let Some(after) = after {
                                transitions += 1;
                                if after.stage().is_terminal() {
                                    return Ok(report(
                                        after,
                                        step_number,
                                        transitions,
                                        recoveries,
                                    ));
                                }
                            }
                            continue;
                        }
                    };
                    match receipt.status() {
                        DispatchStatus::Accepted | DispatchStatus::Settled => {
                            let after = match self.settle_dispatch(
                                port,
                                &mut machine,
                                &observation,
                                &operation_id,
                                &action,
                                &receipt,
                            ) {
                                Ok(after) => after,
                                Err(_) => {
                                    machine.require_recovery(Some(operation_id.clone()));
                                    let after = self.reconcile_uncertain(
                                        port,
                                        &mut machine,
                                        &observation,
                                        &operation_id,
                                    )?;
                                    recoveries += 1;
                                    if let Some(after) = after {
                                        transitions += 1;
                                        if after.stage().is_terminal() {
                                            return Ok(report(
                                                after,
                                                step_number,
                                                transitions,
                                                recoveries,
                                            ));
                                        }
                                    }
                                    continue;
                                }
                            };
                            transitions += 1;
                            if after.stage().is_terminal() {
                                return Ok(report(
                                    after,
                                    step_number,
                                    transitions,
                                    recoveries,
                                ));
                            }
                        }
                        DispatchStatus::Unknown => {
                            machine.require_recovery(Some(operation_id.clone()));
                            let after = self.reconcile_uncertain(
                                port,
                                &mut machine,
                                &observation,
                                &operation_id,
                            )?;
                            recoveries += 1;
                            if let Some(after) = after {
                                transitions += 1;
                                if after.stage().is_terminal() {
                                    return Ok(report(
                                        after,
                                        step_number,
                                        transitions,
                                        recoveries,
                                    ));
                                }
                            }
                        }
                        DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                            machine.require_recovery(Some(operation_id));
                            self.reobserve(port, &mut machine)?;
                            recoveries += 1;
                        }
                    }
                }
                PolicyChoice::Wait { .. } => {
                    let operation_id = format!(
                        "episode-wait-{}-{}",
                        observation.generation(),
                        step_number
                    );
                    match self.config.barrier.await_transition_sample(
                        port,
                        &operation_id,
                        &observation,
                    ) {
                        Ok(sample) => {
                            let after = sample
                                .observation()
                                .cloned()
                                .ok_or(EpisodeRunnerError::MissingObservation)?;
                            accept_observation(&mut machine, after)?;
                        }
                        Err(_) => {
                            self.reobserve(port, &mut machine)?;
                            recoveries += 1;
                        }
                    }
                }
                PolicyChoice::Reobserve { .. } => {
                    self.reobserve(port, &mut machine)?;
                    recoveries += 1;
                }
                PolicyChoice::Recovery { operation, .. } => {
                    match self
                        .config
                        .recovery
                        .recover(port, operation)
                        .map_err(EpisodeRunnerError::Recovery)?
                    {
                        RecoveryResult::Observation(observation) => {
                            accept_observation(&mut machine, observation)?;
                            recoveries += 1;
                        }
                        RecoveryResult::Receipt(receipt) => {
                            if receipt.status() != DispatchStatus::Settled {
                                return Err(EpisodeRunnerError::UncertainMutation);
                            }
                            let after = receipt
                                .after()
                                .cloned()
                                .ok_or(EpisodeRunnerError::MissingObservation)?;
                            verify_settlement(&observation, &receipt)
                                .map_err(EpisodeRunnerError::Postcondition)?;
                            accept_observation(&mut machine, after)?;
                            recoveries += 1;
                        }
                        RecoveryResult::Released | RecoveryResult::Stopped => {
                            machine.fail();
                            return Err(EpisodeRunnerError::StoppedByRecovery);
                        }
                    }
                }
            }
        }
        Err(EpisodeRunnerError::StepLimitExceeded)
    }

    fn reobserve<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
    ) -> Result<EpisodeObservation, EpisodeRunnerError> {
        let result = self
            .config
            .recovery
            .recover(port, RecoveryOperation::Reobserve)
            .map_err(EpisodeRunnerError::Recovery)?;
        let RecoveryResult::Observation(observation) = result else {
            return Err(EpisodeRunnerError::UnexpectedRecoveryResult);
        };
        accept_observation(machine, observation.clone())?;
        Ok(observation)
    }

    fn reconcile_uncertain<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        before: &EpisodeObservation,
        operation_id: &str,
    ) -> Result<Option<EpisodeObservation>, EpisodeRunnerError> {
        let result = self
            .config
            .recovery
            .recover(
                port,
                RecoveryOperation::Reconcile {
                    operation_id: operation_id.to_owned(),
                },
            )
            .map_err(EpisodeRunnerError::Recovery)?;
        let RecoveryResult::Receipt(receipt) = result else {
            return Err(EpisodeRunnerError::UnexpectedRecoveryResult);
        };
        if receipt.operation_id() != operation_id {
            return Err(EpisodeRunnerError::ConflictingOperation);
        }
        match receipt.status() {
            DispatchStatus::Settled => {
                let after = receipt
                    .after()
                    .cloned()
                    .ok_or(EpisodeRunnerError::MissingObservation)?;
                verify_settlement(before, &receipt)
                    .map_err(EpisodeRunnerError::Postcondition)?;
                accept_observation(machine, after.clone())?;
                Ok(Some(after))
            }
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                self.reobserve(port, machine)?;
                Ok(None)
            }
            DispatchStatus::Accepted | DispatchStatus::Unknown => {
                Err(EpisodeRunnerError::UncertainMutation)
            }
        }
    }

    fn settle_dispatch<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        before: &EpisodeObservation,
        operation_id: &str,
        action: &EpisodeLegalAction,
        receipt: &TransitionReceipt,
    ) -> Result<EpisodeObservation, EpisodeRunnerError> {
        if receipt.operation_id() != operation_id
            || receipt.action().action_id() != action.action_id()
        {
            return Err(EpisodeRunnerError::ConflictingOperation);
        }
        let sample = self
            .config
            .barrier
            .await_transition_sample(port, operation_id, before)
            .map_err(EpisodeRunnerError::Barrier)?;
        let after = sample
            .observation()
            .cloned()
            .ok_or(EpisodeRunnerError::MissingObservation)?;
        let effect_kind = receipt
            .effect_kind()
            .or(sample.effect_kind())
            .map(str::to_owned)
            .ok_or(EpisodeRunnerError::MissingEffectWitness)?;
        let settled = TransitionReceipt::new(
            operation_id,
            action.clone(),
            DispatchStatus::Settled,
            Some(after.clone()),
            Some(effect_kind),
            None,
        );
        verify_settlement(before, &settled).map_err(EpisodeRunnerError::Postcondition)?;
        machine
            .settle(after.clone())
            .map_err(EpisodeRunnerError::Machine)?;
        Ok(after)
    }
}

fn accept_observation(
    machine: &mut EpisodeMachine,
    observation: EpisodeObservation,
) -> Result<(), EpisodeRunnerError> {
    machine.observe(observation).map_err(|error| match error {
        EpisodeMachineError::UnknownState | EpisodeMachineError::StaleObservation => {
            EpisodeRunnerError::RecoveryRequired
        }
        other => EpisodeRunnerError::Machine(other),
    })
}

fn report(
    final_observation: EpisodeObservation,
    steps: u32,
    transitions: u32,
    recoveries: u32,
) -> EpisodeRunReport {
    EpisodeRunReport {
        terminal_stage: final_observation.stage(),
        steps,
        transitions,
        recoveries,
        final_observation,
    }
}
