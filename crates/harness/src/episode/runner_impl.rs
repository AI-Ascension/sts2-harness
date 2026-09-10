// SPDX-License-Identifier: MIT

use super::super::idempotency::ActionLedger;
use super::super::policy_router::DecisionSource;
use super::super::state_machine::EpisodeMachine;
use super::runner_steps::RunCounters;
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};

pub(super) struct RunFailure {
    pub(super) error: EpisodeRunnerError,
    pub(super) pending_operation_id: Option<String>,
}

impl EpisodeRunner {
    pub(super) fn run_inner<P: EpisodeRuntimePort, S: DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
    ) -> Result<EpisodeRunReport, RunFailure> {
        let mut machine = EpisodeMachine::new();
        let mut ledger =
            ActionLedger::new(self.config.max_steps as usize).map_err(|error| RunFailure {
                error: EpisodeRunnerError::Ledger(error),
                pending_operation_id: None,
            })?;
        let mut counters = RunCounters::default();
        for step in 0..self.config.max_steps {
            match self.run_step(port, source, &mut machine, &mut ledger, step, &mut counters) {
                Ok(Some(report)) => return Ok(report),
                Ok(None) => {}
                Err(error) => {
                    return Err(RunFailure {
                        error,
                        pending_operation_id: machine.pending_operation_id().map(str::to_owned),
                    });
                }
            }
        }
        Err(RunFailure {
            error: EpisodeRunnerError::StepLimitExceeded,
            pending_operation_id: machine.pending_operation_id().map(str::to_owned),
        })
    }
}
