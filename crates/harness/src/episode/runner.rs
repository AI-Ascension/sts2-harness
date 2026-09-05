// SPDX-License-Identifier: MIT

#[path = "runner_actions.rs"]
mod runner_actions;
#[path = "runner_error.rs"]
mod runner_error;
#[path = "runner_impl.rs"]
mod runner_impl;
#[path = "runner_recovery.rs"]
mod runner_recovery;
#[path = "runner_steps.rs"]
mod runner_steps;

use super::idempotency::ActionIdentity;
use super::legal_actions::{EpisodeLegalAction, EpisodeLegalActionSet};
use super::observation::{EpisodeObservation, EpisodeStage};
use super::recovery::{RecoveryController, RecoveryPort};
use super::shutdown::{EpisodeShutdown, ShutdownPort};
use super::stability_barrier::{BarrierPort, StabilityBarrier};
use super::transition::TransitionReceipt;
use crate::error::PortError;

pub use runner_error::EpisodeRunnerError;

const MAX_STEPS: u32 = 1_024;
const MAX_OBJECTIVE_BYTES: usize = 512;
const MAX_CONSTRAINTS: usize = 32;

/// Runtime port assembled by the harness from the gateway and MCP adapters.
///
/// The harness owns this orchestration port, but the implementation remains responsible for
/// routing every request through the gateway/MCP path. It never exposes a game-process handle or
/// an alternate action authority to the runner.
pub trait EpisodeRuntimePort: BarrierPort + RecoveryPort + ShutdownPort {
    fn launch(&mut self) -> Result<(), PortError>;

    fn observe(&mut self) -> Result<EpisodeObservation, PortError>;

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError>;

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError>;
}

/// Bounded policy and transition settings for one complete episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeRunnerConfig {
    max_steps: u32,
    barrier: StabilityBarrier,
    recovery: RecoveryController,
    objective: String,
    hard_constraints: Vec<String>,
}

impl EpisodeRunnerConfig {
    pub fn new(
        max_steps: u32,
        barrier: StabilityBarrier,
        recovery: RecoveryController,
        objective: impl Into<String>,
        hard_constraints: Vec<String>,
    ) -> Result<Self, EpisodeRunnerError> {
        let objective = objective.into();
        if max_steps == 0
            || max_steps > MAX_STEPS
            || !valid_text(&objective, MAX_OBJECTIVE_BYTES)
            || hard_constraints.len() > MAX_CONSTRAINTS
            || hard_constraints
                .iter()
                .any(|constraint| !valid_text(constraint, MAX_OBJECTIVE_BYTES))
        {
            return Err(EpisodeRunnerError::InvalidConfiguration);
        }
        Ok(Self {
            max_steps,
            barrier,
            recovery,
            objective,
            hard_constraints,
        })
    }

    #[must_use]
    pub const fn max_steps(&self) -> u32 {
        self.max_steps
    }

    #[must_use]
    pub fn objective(&self) -> &str {
        &self.objective
    }

    #[must_use]
    pub fn hard_constraints(&self) -> &[String] {
        &self.hard_constraints
    }
}

/// Terminal result and bounded counters from a completed or defeated run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeRunReport {
    terminal_stage: EpisodeStage,
    steps: u32,
    transitions: u32,
    recoveries: u32,
    final_observation: EpisodeObservation,
}

impl EpisodeRunReport {
    #[must_use]
    pub const fn terminal_stage(&self) -> EpisodeStage {
        self.terminal_stage
    }

    #[must_use]
    pub const fn steps(&self) -> u32 {
        self.steps
    }

    #[must_use]
    pub const fn transitions(&self) -> u32 {
        self.transitions
    }

    #[must_use]
    pub const fn recoveries(&self) -> u32 {
        self.recoveries
    }

    #[must_use]
    pub fn final_observation(&self) -> &EpisodeObservation {
        &self.final_observation
    }
}

/// Harness-owned complete-run coordinator. It has no gameplay heuristic or fallback action path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeRunner {
    config: EpisodeRunnerConfig,
}

impl EpisodeRunner {
    #[must_use]
    pub const fn new(config: EpisodeRunnerConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(&self) -> &EpisodeRunnerConfig {
        &self.config
    }

    /// Launches through the runtime port, runs until victory/defeat or a bounded failure, and
    /// always attempts lease, MCP, and gateway cleanup after launch succeeds.
    pub fn run<P: EpisodeRuntimePort, S: super::policy_router::DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
    ) -> Result<EpisodeRunReport, EpisodeRunnerError> {
        port.launch().map_err(EpisodeRunnerError::Launch)?;
        let outcome = self.run_inner(port, source);
        match EpisodeShutdown.close(port) {
            Ok(()) => outcome,
            Err(error) => Err(EpisodeRunnerError::Shutdown(error)),
        }
    }
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
