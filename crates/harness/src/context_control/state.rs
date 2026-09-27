// SPDX-License-Identifier: MIT

//! The control authority: the one owner of pause, commit, resume, and stop for a run.
//!
//! The types and the remaining transitions live here. The gate transitions are in
//! `state/gate.rs`, operation admission in `state/operations.rs`, the journal boundary in
//! `state/journal.rs`, the idempotency and event-ledger bookkeeping in `state/ledger.rs`, and the
//! digests they all depend on in `state/digest.rs`.

use super::types::ContextBoundary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod digest;
mod gate;
mod journal;
mod ledger;
mod operations;

use digest::digest;

/// Outer ceiling on recorded control transitions for one run.
///
/// The selected owner/profile advertises `max_control_events` and may narrow this bound; the
/// harness maximum is the outer ceiling and is never raised. A journal that retains more
/// transitions than the bound is refused at recovery, and a narrowed authority refuses to record
/// beyond its selected bound instead of silently dropping the transition.
pub const MAX_CONTROL_EVENTS: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GateStatus {
    Running,
    PauseRequested,
    PausedReady,
    PausedStale,
    PausedCommitted,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    pub status: GateStatus,
    pub control_version: u64,
    pub gate_epoch: u64,
    pub plan_epoch: u64,
    pub active_revision_id: String,
    pub pause_latched: bool,
    pub stop_latched: bool,
    pub unresolved_operations: Vec<String>,
    pub boundary: ContextBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlReceipt {
    pub command_id: String,
    pub idempotency_key: String,
    pub effect: String,
    pub control_version: u64,
    pub plan_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlEvent {
    pub sequence: u64,
    pub event_type: String,
    pub command_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlAuthority {
    owner_epoch: u64,
    state: ControlState,
    commands: BTreeMap<String, (String, ControlReceipt)>,
    events: Vec<ControlEvent>,
    next_id: u64,
    max_control_events: u64,
}

impl ControlAuthority {
    pub fn new(boundary: ContextBoundary, active_revision_id: impl Into<String>) -> Self {
        let gate_epoch = boundary.gate_epoch;
        let control_version = boundary.control_version;
        Self {
            owner_epoch: boundary.controller_epoch,
            state: ControlState {
                status: GateStatus::Running,
                control_version,
                gate_epoch,
                plan_epoch: 1,
                active_revision_id: active_revision_id.into(),
                pause_latched: false,
                stop_latched: false,
                unresolved_operations: Vec::new(),
                boundary,
            },
            commands: BTreeMap::new(),
            events: Vec::new(),
            next_id: 1,
            max_control_events: MAX_CONTROL_EVENTS,
        }
    }

    pub fn state(&self) -> &ControlState {
        &self.state
    }

    pub fn events(&self) -> &[ControlEvent] {
        &self.events
    }

    /// The selected bound on recorded control transitions for this authority.
    #[must_use]
    pub fn max_control_events(&self) -> u64 {
        self.max_control_events
    }

    /// Narrows recorded control transitions to the selected owner/profile bound.
    ///
    /// The harness maximum stays the outer ceiling, so a selected profile can only narrow:
    /// `limit` must be non-zero and no greater than [`MAX_CONTROL_EVENTS`]. A refusal names the
    /// exact reason and leaves the authority unchanged.
    pub fn with_max_control_events(mut self, limit: u64) -> Result<Self, String> {
        if limit == 0 || limit > MAX_CONTROL_EVENTS {
            return Err("control_event_limit_invalid".to_owned());
        }
        if self.events.len() as u64 > limit {
            return Err("context_control_events_exhausted".to_owned());
        }
        self.max_control_events = limit;
        Ok(self)
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.reserve_events(1)?;
        self.state.stop_latched = true;
        self.state.pause_latched = true;
        self.state.status = GateStatus::Stopped;
        self.state.gate_epoch = self.state.gate_epoch.saturating_add(1);
        self.bump_boundary();
        self.event("stop.latched", None, Some("stop_dominates_resume"))?;
        Ok(())
    }

    pub fn advance_boundary(&mut self) -> Result<(), String> {
        self.reserve_events(1)?;
        self.state.boundary.generation = self.state.boundary.generation.saturating_add(1);
        self.state.boundary.observation_sha256 =
            digest(format!("observation-{}", self.state.boundary.generation).as_bytes());
        if self.state.pause_latched {
            self.state.status = GateStatus::PausedStale;
        }
        self.event("boundary.changed", None, Some("requires_reobserve"))?;
        Ok(())
    }

    /// Replaces the game-observation portion of this authority's boundary
    /// from its owning runtime. The authority keeps its own controller, gate,
    /// and control epochs; a gateway/MCP observation cannot mint control
    /// authority or move the boundary backwards.
    pub fn record_observation_boundary(
        &mut self,
        mut observed: ContextBoundary,
    ) -> Result<(), String> {
        let current = &self.state.boundary;
        if observed.run_id != current.run_id
            || observed.episode_id != current.episode_id
            || observed.agent_id != current.agent_id
            || observed.generation < current.generation
            || (observed.generation == current.generation
                && (observed.state_id != current.state_id
                    || observed.observation_sha256 != current.observation_sha256))
        {
            return Err("observation_boundary_stale".to_owned());
        }
        observed.controller_epoch = current.controller_epoch;
        observed.gate_epoch = self.state.gate_epoch;
        observed.control_version = self.state.control_version;
        self.state.boundary = observed;
        Ok(())
    }

    pub fn admit_plan(&self, plan_epoch: u64) -> Result<(), String> {
        if self.state.pause_latched
            || self.state.stop_latched
            || plan_epoch != self.state.plan_epoch
        {
            return Err("obsolete_plan".to_owned());
        }
        Ok(())
    }
}
