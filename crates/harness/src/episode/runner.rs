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
use super::runtime_lease_binding::RuntimeLeaseBinding;
use super::shutdown::{EpisodeShutdown, ShutdownPort};
use super::stability_barrier::{BarrierPort, StabilityBarrier};
use super::transition::TransitionReceipt;
use crate::error::PortError;
use crate::game_information::LookupAgentPort;
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

    /// Returns the lease identity actually installed by the runtime after
    /// launch allocation. Implementations without an authoritative allocation
    /// handoff fail closed when a consumer requires this provenance.
    fn current_lease_binding(&mut self) -> Result<RuntimeLeaseBinding, PortError> {
        Err(PortError::new(
            "runtime_lease_binding_unavailable",
            "runtime did not expose its post-launch gateway lease binding",
            false,
        ))
    }

    /// Runs the additive selected-policy tool loop against the already-open
    /// owner and existing MCP session. Adapters without that opt-in path fail closed.
    fn run_game_information_lookup(
        &mut self,
        _legal_actions: &EpisodeLegalActionSet,
        _agent: &mut dyn LookupAgentPort,
    ) -> Result<String, crate::episode::PolicyError> {
        Err(crate::episode::PolicyError::ProviderUnavailable)
    }

    /// Resolves the enabled owner-backed game-information binding after lease
    /// admission and before the runner reads an episode observation.
    ///
    /// Adapters without the additive LBR v1 route remain compatible by
    /// inheriting the no-op implementation. An enabled adapter must fail
    /// closed rather than substituting locally derived game information.
    fn prepare_game_information_binding(&mut self) -> Result<(), PortError> {
        Ok(())
    }

    /// Refreshes an enabled owner-issued game-information observation before a
    /// provider can make a decision from the matching observation generation.
    ///
    /// Adapters without the additive lookup-binding route remain compatible.
    /// Enabled adapters must reject stale, mixed, or unavailable observations
    /// before invoking a decision source.
    fn refresh_game_information_binding(
        &mut self,
        _state_id: &str,
        _generation: u64,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError>;

    /// Reads the authored projection binding through the runtime boundary.
    ///
    /// Existing runtime adapters may inherit the ordinary observation path while
    /// newer adapters can route the reference to a negotiated projection.
    fn observe_projection(
        &mut self,
        projection_ref: &str,
    ) -> Result<EpisodeObservation, PortError> {
        Err(PortError::new(
            "projection_binding_unavailable",
            format!("runtime does not support authored projection {projection_ref}"),
            false,
        ))
    }

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
    max_consecutive_abstentions: u8,
    max_repeated_situations: u16,
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
            max_consecutive_abstentions: 0,
            max_repeated_situations: 0,
        })
    }

    /// Bounds how many times one game situation may be reached before the episode is abandoned.
    ///
    /// Zero, the default, keeps the previous behaviour: a cycle runs until the step or time limit.
    /// Above zero, a situation reached that many times ends the episode with
    /// [`EpisodeRunnerError::RepeatedSituation`].
    ///
    /// A situation is the observation with its `state_id` and `generation` removed, because those
    /// advance on every step and would make a repeat look new. Confident decisions can still cycle:
    /// taking a reward, failing to rank its cards, skipping, and being offered the same reward
    /// again is a loop of decisions that each clear the confidence gate, so the abstention bound
    /// does not see it.
    #[must_use]
    pub const fn with_max_repeated_situations(mut self, bound: u16) -> Self {
        self.max_repeated_situations = bound;
        self
    }

    #[must_use]
    pub const fn max_repeated_situations(&self) -> u16 {
        self.max_repeated_situations
    }

    /// Bounds how many times an unchanged state may be re-asked before the runner settles.
    ///
    /// Zero, the default, keeps the previous behaviour: an abstention always observes again, for as
    /// many steps as the episode has. Above zero, once that many consecutive abstentions have been
    /// made on one `state_id` and `generation`, the runner dispatches the candidate the source
    /// carried instead of asking a fourth time. It settles only on a candidate the source named and
    /// the host still offers; with no candidate it observes again exactly as before.
    #[must_use]
    pub const fn with_max_consecutive_abstentions(mut self, bound: u8) -> Self {
        self.max_consecutive_abstentions = bound;
        self
    }

    #[must_use]
    pub const fn max_consecutive_abstentions(&self) -> u8 {
        self.max_consecutive_abstentions
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
        let outcome = match port.prepare_game_information_binding() {
            Ok(()) => self.run_inner(port, source),
            Err(error) => Err(runner_impl::RunFailure {
                error: EpisodeRunnerError::GameInformationBinding(error),
                pending_operation_id: None,
            }),
        };
        // Only a successful terminal outcome is a completed episode; cleanup
        // must not arm a repeated-episode profile for a failed run.
        if outcome.is_ok() {
            port.mark_episode_completed();
        }
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

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
