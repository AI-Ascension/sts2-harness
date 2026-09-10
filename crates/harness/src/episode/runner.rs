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
use super::protected::ProtectedEpisodePort;
use super::recovery::{RecoveryController, RecoveryPort};
use super::shutdown::{EpisodeShutdown, ShutdownPort};
use super::stability_barrier::{BarrierPort, StabilityBarrier};
use super::transition::TransitionReceipt;
use crate::error::PortError;
use crate::identity::ModelExecutionId;
use serde_json::Value;

pub use runner_error::{EpisodeRunFailure, EpisodeRunnerError};

const MAX_STEPS: u32 = 4_096;
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

    /// Reads one generation-bound map projection through the runtime MCP boundary.
    /// Implementations may return `None` when the active runtime does not expose map visibility.
    fn map_snapshot(
        &mut self,
        _state_id: &str,
        _generation: u64,
        _execution_id: ModelExecutionId,
    ) -> Result<Option<Value>, PortError> {
        Ok(None)
    }

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
    map_context_enabled: bool,
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
            map_context_enabled: false,
        })
    }

    /// Enables the negotiated map projection for this runner. When enabled, a map-stage
    /// observation must receive a validated map snapshot; the runner never falls back to the
    /// ordinary decision schema after this opt-in.
    #[must_use]
    pub fn with_map_context_enabled(mut self, enabled: bool) -> Self {
        self.map_context_enabled = enabled;
        self
    }

    #[must_use]
    pub const fn map_context_enabled(&self) -> bool {
        self.map_context_enabled
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
    pub fn run<P: ProtectedEpisodePort, S: super::policy_router::DecisionSource>(
        &self,
        port: &mut P,
        source: &mut S,
    ) -> Result<EpisodeRunReport, EpisodeRunnerError> {
        port.launch().map_err(EpisodeRunnerError::Launch)?;
        let outcome = self.run_inner(port, source);
        let cleanup = EpisodeShutdown.close_report(port);
        match (outcome, cleanup.first_failure()) {
            (Ok(report), None) => Ok(report),
            (Ok(_), Some(error)) => Err(EpisodeRunnerError::Shutdown(error)),
            (Err(failure), None) => Err(failure.error),
            (Err(failure), Some(_)) => Err(EpisodeRunnerError::Cleanup(EpisodeRunFailure::new(
                failure.error,
                cleanup,
                failure.pending_operation_id,
            ))),
        }
    }
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
