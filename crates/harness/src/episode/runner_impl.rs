// SPDX-License-Identifier: MIT

use super::super::idempotency::ActionLedger;
use super::super::policy_router::DecisionSource;
use super::super::state_machine::EpisodeMachine;
use super::runner_steps::RunCounters;
use super::{EpisodeRunReport, EpisodeRunner, EpisodeRunnerError, EpisodeRuntimePort};

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
}
