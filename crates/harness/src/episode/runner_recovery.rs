// SPDX-License-Identifier: MIT

use super::super::legal_actions::EpisodeLegalAction;
use super::super::observation::EpisodeObservation;
use super::super::postconditions::verify_settlement;
use super::super::recovery::{RecoveryOperation, RecoveryResult};
use super::super::state_machine::{EpisodeMachine, EpisodeMachineError};
use super::super::transition::{DispatchStatus, TransitionReceipt};
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};

impl EpisodeRunner {
    pub(super) fn reobserve<P: EpisodeRuntimePort>(
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

    pub(super) fn reconcile_uncertain<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        before: &EpisodeObservation,
        operation_id: &str,
        action: &EpisodeLegalAction,
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
        if receipt.operation_id() != operation_id || receipt.action() != action {
            return Err(EpisodeRunnerError::ConflictingOperation);
        }
        match receipt.status() {
            DispatchStatus::Settled => {
                let after = receipt
                    .after()
                    .cloned()
                    .ok_or(EpisodeRunnerError::MissingObservation)?;
                verify_settlement(before, &receipt).map_err(EpisodeRunnerError::Postcondition)?;
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

    pub(super) fn settle_dispatch<P: EpisodeRuntimePort>(
        &self,
        port: &mut P,
        machine: &mut EpisodeMachine,
        before: &EpisodeObservation,
        operation_id: &str,
        action: &EpisodeLegalAction,
        receipt: &TransitionReceipt,
    ) -> Result<EpisodeObservation, EpisodeRunnerError> {
        if receipt.operation_id() != operation_id || receipt.action() != action {
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

pub(super) fn accept_observation(
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

pub(super) fn report(
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
