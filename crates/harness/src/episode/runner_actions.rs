// SPDX-License-Identifier: MIT

use super::super::idempotency::{ActionLedger, Admission};
use super::super::legal_actions::EpisodeLegalActionSet;
use super::super::observation::EpisodeObservation;
use super::super::postconditions::verify_settlement;
use super::super::recovery::{RecoveryOperation, RecoveryResult};
use super::super::state_machine::EpisodeMachine;
use super::super::transition::{DispatchStatus, TransitionReceipt};
use super::runner_recovery::{accept_observation, report};
use super::runner_steps::{ActionExecution, RunCounters};
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};
use crate::episode::idempotency::ActionIdentity;

pub(super) struct ActionRequest<'a> {
    pub(super) observation: &'a EpisodeObservation,
    pub(super) legal_actions: &'a EpisodeLegalActionSet,
    pub(super) action_id: &'a str,
    pub(super) step_number: u32,
}

struct ActionContext<'a> {
    observation: &'a EpisodeObservation,
    execution: &'a ActionExecution,
    step_number: u32,
}

impl EpisodeRunner {
    pub(super) fn handle_action<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        ledger: &mut ActionLedger,
        request: ActionRequest<'_>,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        let execution = prepare_action(
            machine,
            ledger,
            request.observation,
            request.legal_actions,
            request.action_id,
            request.step_number,
        )?;
        let receipt = match port.dispatch_action(&execution.identity, &execution.action) {
            Ok(receipt) => receipt,
            Err(_error) => {
                let context = ActionContext {
                    observation: request.observation,
                    execution: &execution,
                    step_number: request.step_number,
                };
                return self.recover_action(port, machine, context, counters);
            }
        };
        let context = ActionContext {
            observation: request.observation,
            execution: &execution,
            step_number: request.step_number,
        };
        self.handle_receipt(port, machine, context, receipt, counters)
    }

    fn handle_receipt<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        context: ActionContext<'_>,
        receipt: TransitionReceipt,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        match receipt.status() {
            DispatchStatus::Accepted | DispatchStatus::Settled => {
                match self.settle_dispatch(
                    port,
                    machine,
                    context.observation,
                    &context.execution.operation_id,
                    &context.execution.action,
                    &receipt,
                ) {
                    Ok(after) => complete_transition(after, context.step_number, counters),
                    Err(_) => self.recover_action(port, machine, context, counters),
                }
            }
            DispatchStatus::Unknown => self.recover_action(port, machine, context, counters),
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                machine.require_recovery(Some(context.execution.operation_id.clone()));
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
        context: ActionContext<'_>,
        counters: &mut RunCounters,
    ) -> Result<Option<EpisodeRunReport>, EpisodeRunnerError> {
        machine.require_recovery(Some(context.execution.operation_id.clone()));
        let after = self.reconcile_uncertain(
            port,
            machine,
            context.observation,
            &context.execution.operation_id,
            &context.execution.action,
        )?;
        counters.recoveries += 1;
        Ok(after.and_then(|after| {
            counters.transitions += 1;
            after.stage().is_terminal().then(|| {
                report(
                    after,
                    context.step_number,
                    counters.transitions,
                    counters.recoveries,
                )
            })
        }))
    }

    pub(super) fn handle_wait<P: EpisodeRuntimePort>(
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

    pub(super) fn handle_recovery<P: EpisodeRuntimePort>(
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
