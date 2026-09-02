// SPDX-License-Identifier: MIT

mod contract;
mod doubles;
mod response;
mod trace;
mod wire;

pub use contract::{PocAction, PocCoreError, PocError, PocObservation, PocStatus};
pub use trace::TraceEvent;

use crate::protocol_artifact::{
    POC_ACTION_RESPONSE, POC_ARTIFACT, POC_GENERATOR, POC_INVALID_ACTION, POC_PROTOCOL_VERSION,
    POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE, POC_STATE_RESPONSE, verify_poc_artifact,
};
use contract::PocRequest;
use doubles::McpDouble;
use trace::TraceLedger;

const INSTANCE_ID: &str = "instance-1";
const SESSION_ID: &str = "session-1";
const LEASE_ID: &str = "lease-1";
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
    session_id: &'static str,
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

    /// Returns the session identity carried through the fake boundary path.
    #[must_use]
    pub const fn session_id(&self) -> &str {
        self.session_id
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PocRunner;

impl PocRunner {
    /// Creates the fixed deterministic POC runner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Executes state read, accepted action, and rejected action through local fakes.
    pub fn run(&self) -> Result<PocReport, PocError> {
        verify_poc_artifact().map_err(PocError::Artifact)?;
        let mut mcp = McpDouble::new(POC_SEED, POC_CLOCK_TICK);
        let mut trace = TraceLedger::new();

        let state_token = trace.enter("harness", "get_state", SESSION_ID, LEASE_ID)?;
        let state = mcp.get_state(SESSION_ID, INSTANCE_ID, "corr-0001", &mut trace)?;
        if state.wire_json()?.trim() != POC_STATE_RESPONSE.trim() {
            return Err(PocError::InvalidTrace(
                "state response does not match the copied golden wire message",
            ));
        }
        trace.complete(state_token, &state)?;

        let accepted_request = PocRequest::action_request(
            "corr-0002",
            INSTANCE_ID,
            SESSION_ID,
            0,
            PocAction::new("use_budget", 1),
            LEASE_ID,
        );
        let accepted_token = trace.enter("harness", "submit_action", SESSION_ID, LEASE_ID)?;
        let accepted = mcp.submit_action(accepted_request, &mut trace)?;
        if accepted.wire_json()?.trim() != POC_ACTION_RESPONSE.trim() {
            return Err(PocError::InvalidTrace(
                "accepted response does not match the copied golden wire message",
            ));
        }
        trace.complete(accepted_token, &accepted)?;

        let rejected_request = PocRequest::action_request(
            "corr-0003",
            INSTANCE_ID,
            SESSION_ID,
            1,
            PocAction::new("use_budget", 0),
            LEASE_ID,
        );
        let rejected_token = trace.enter("harness", "submit_action", SESSION_ID, LEASE_ID)?;
        let rejected = mcp.submit_action(rejected_request.clone(), &mut trace)?;
        if rejected_request.wire_json()?.trim() != POC_INVALID_ACTION.trim() {
            return Err(PocError::InvalidTrace(
                "rejected action request does not match the copied fixture",
            ));
        }
        trace.complete(rejected_token, &rejected)?;
        let trace = trace.finish()?;

        let first_trace = trace
            .first()
            .ok_or(PocError::InvalidTrace("state trace is missing"))?;
        let initial = first_trace.observation();
        let initial_generation = first_trace.generation();
        let accepted_observation = accepted.observation();
        let rejected_observation = rejected.observation();
        let accepted_changed_once = accepted.status() == Some(PocStatus::Accepted)
            && accepted.error_code().is_none()
            && accepted_observation != initial
            && accepted.generation() == initial_generation.saturating_add(1)
            && accepted_observation.available_units == 2
            && accepted_observation.settled_effects == 1;
        let rejected_unchanged = rejected.status() == Some(PocStatus::Rejected)
            && rejected.error_code() == Some("sts2.game-core/zero_units")
            && rejected_observation == accepted_observation
            && rejected.generation() == accepted.generation();
        if !accepted_changed_once {
            return Err(PocError::InvalidTrace(
                "accepted action did not change state exactly once",
            ));
        }
        if !rejected_unchanged {
            return Err(PocError::InvalidTrace(
                "rejected action changed the bounded state",
            ));
        }
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
            session_id: SESSION_ID,
            seed: POC_SEED,
            clock_tick: POC_CLOCK_TICK,
            accepted_changed_once,
            rejected_unchanged,
        })
    }
}
/// Runs the POC with its fixed seed and clock.
pub fn run_poc() -> Result<PocReport, PocError> {
    PocRunner::new().run()
}
