// SPDX-License-Identifier: MIT

mod contract;
mod doubles;
mod trace;

pub use contract::{PocAction, PocCoreError, PocError, PocObservation, PocStatus};
pub use trace::TraceEvent;

use crate::protocol_artifact::{
    POC_ARTIFACT, POC_GENERATOR, POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE,
    verify_poc_artifact,
};
use contract::PocResponse;
use doubles::McpDouble;

const INSTANCE_ID: &str = "instance-1";
const SESSION_ID: &str = "session-1";
const LEASE_ID: &str = "lease-1";
const BOUNDARIES: [&str; 5] = ["harness", "mcp", "gateway", "game-mod", "game-core"];

/// Fixed seed used by the deterministic fake runner.
pub const POC_SEED: u64 = 7;
/// Fixed monotonic clock value used by the deterministic fake runner.
pub const POC_CLOCK_TICK: u64 = 0;

/// Output of the one deterministic fake vertical slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PocReport {
    trace: Vec<TraceEvent>,
    trace_bytes: String,
    artifact_lineage: String,
    seed: u64,
    clock_tick: u64,
    accepted_changed_once: bool,
    rejected_unchanged: bool,
}

impl PocReport {
    /// Returns one event for each boundary crossed by each of the three operations.
    #[must_use]
    pub fn trace(&self) -> &[TraceEvent] {
        &self.trace
    }

    /// Returns canonical newline-delimited trace bytes.
    #[must_use]
    pub fn trace_bytes(&self) -> &str {
        &self.trace_bytes
    }

    /// Returns the protocol artifact lineage recorded by this run.
    #[must_use]
    pub fn artifact_lineage(&self) -> &str {
        &self.artifact_lineage
    }

    /// Returns the fixed runner seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the fixed runner clock value.
    #[must_use]
    pub const fn clock_tick(&self) -> u64 {
        self.clock_tick
    }

    /// Reports whether the accepted action changed the bounded state exactly once.
    #[must_use]
    pub const fn accepted_changed_once(&self) -> bool {
        self.accepted_changed_once
    }

    /// Reports whether the rejected action left the bounded state unchanged.
    #[must_use]
    pub const fn rejected_unchanged(&self) -> bool {
        self.rejected_unchanged
    }
}

/// Configures and executes the deterministic fake POC runner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PocRunner {
    seed: u64,
    clock_tick: u64,
}

impl Default for PocRunner {
    fn default() -> Self {
        Self::new(POC_SEED, POC_CLOCK_TICK)
    }
}

impl PocRunner {
    /// Creates a runner with explicit deterministic inputs.
    #[must_use]
    pub const fn new(seed: u64, clock_tick: u64) -> Self {
        Self { seed, clock_tick }
    }

    /// Executes state read, accepted action, and rejected action through local fakes.
    pub fn run(&self) -> Result<PocReport, PocError> {
        verify_poc_artifact().map_err(PocError::Artifact)?;
        let mut mcp = McpDouble::new();
        let mut trace = Vec::new();

        let state = mcp.get_state(SESSION_ID, INSTANCE_ID, "corr-0001")?;
        append_trace(&mut trace, "get_state", &state);

        let accepted =
            mcp.submit_action(SESSION_ID, INSTANCE_ID, "corr-0002", 0, "use_budget", 1)?;
        append_trace(&mut trace, "submit_action", &accepted);

        let rejected =
            mcp.submit_action(SESSION_ID, INSTANCE_ID, "corr-0003", 1, "use_budget", 0)?;
        append_trace(&mut trace, "submit_action", &rejected);

        let first_trace = trace
            .first()
            .ok_or(PocError::InvalidTrace("state trace is missing"))?;
        let initial = first_trace.observation();
        let initial_generation = first_trace.generation();
        let accepted_observation = accepted.observation();
        let rejected_observation = rejected.observation();
        let accepted_changed_once = accepted_observation != initial
            && accepted.generation() == initial_generation.saturating_add(1)
            && accepted_observation.available_units == 2
            && accepted_observation.settled_effects == 1;
        let rejected_unchanged = rejected_observation == accepted_observation
            && rejected.generation() == accepted.generation();
        let trace_bytes = trace
            .iter()
            .map(TraceEvent::to_json)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";

        Ok(PocReport {
            trace,
            trace_bytes,
            artifact_lineage: format!(
                "artifact={POC_ARTIFACT};protocol_version={POC_PROTOCOL_VERSION};schema_digest={POC_SCHEMA_DIGEST};source={POC_SCHEMA_SOURCE};generator={POC_GENERATOR}"
            ),
            seed: self.seed,
            clock_tick: self.clock_tick,
            accepted_changed_once,
            rejected_unchanged,
        })
    }
}

/// Runs the POC with its fixed seed and clock.
pub fn run_poc() -> Result<PocReport, PocError> {
    PocRunner::default().run()
}

fn append_trace(trace: &mut Vec<TraceEvent>, tool: &'static str, response: &PocResponse) {
    trace.extend(
        BOUNDARIES
            .iter()
            .map(|boundary| response.trace_event(boundary, tool, LEASE_ID)),
    );
}
