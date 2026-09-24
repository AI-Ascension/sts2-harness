// SPDX-License-Identifier: MIT

//! A per-generation bounded readiness wait.
//!
//! A wait is started for one instance, authority epoch and process generation.
//! It admits observations through a bounded deadline and attempt budget, and it
//! settles only from an observation that is bound to that same instance and
//! epoch, labelled with the current generation, and reporting a milestone that
//! reaches the target. A foreign, stale or below-target observation cannot
//! settle it, and a restart invalidates prior readiness so fresh proof is
//! required.

use crate::management::GameplayReadinessEvidence;

use super::{ReadinessMilestone, ReadinessTarget, ReadinessWaitError};

/// How an admitted observation moved the wait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessProgress {
    /// The observation reached the target milestone and settled the wait.
    Satisfied,
    /// The observation was fresh and correctly bound but still below the target.
    AwaitingMore,
}

/// The terminal outcome of a wait, for explicit workflow routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessTerminal {
    /// An authoritative observation reached the target milestone.
    Satisfied,
    /// The owning lifecycle surface denied readiness.
    Denied,
    /// The workflow cancelled the wait.
    Cancelled,
    /// A restart invalidated prior readiness before the target was reached.
    Invalidated,
    /// The bounded deadline or attempt budget was exhausted.
    TimedOut,
}

/// A bounded wait for one readiness milestone, bound to one process generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadinessWait {
    instance_id: String,
    authority_epoch: u64,
    generation: u64,
    target: ReadinessTarget,
    attempts: u32,
    terminal: Option<ReadinessTerminal>,
}

impl ReadinessWait {
    /// Starts a bounded wait for the current instance, epoch and generation.
    ///
    /// Refuses a missing instance identity, a zero authority epoch, a zero
    /// generation ([`ReadinessWaitError::InvalidBinding`]) or an invalid target
    /// before any observation is admitted.
    pub fn begin(
        instance_id: impl Into<String>,
        authority_epoch: u64,
        generation: u64,
        target: ReadinessTarget,
    ) -> Result<Self, ReadinessWaitError> {
        let instance_id = instance_id.into();
        if instance_id.is_empty() || authority_epoch == 0 || generation == 0 {
            return Err(ReadinessWaitError::InvalidBinding);
        }
        Ok(Self {
            instance_id,
            authority_epoch,
            generation,
            target: target.validate()?,
            attempts: 0,
            terminal: None,
        })
    }

    /// The admitted target.
    #[must_use]
    pub const fn target(&self) -> ReadinessTarget {
        self.target
    }

    /// The process generation this wait is bound to.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The number of observations admitted so far.
    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    /// The terminal outcome, if the wait has settled.
    #[must_use]
    pub const fn terminal(&self) -> Option<ReadinessTerminal> {
        self.terminal
    }

    /// Whether the wait has reached any terminal outcome.
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        self.terminal.is_some()
    }

    /// Admits one owner-observed milestone at `now_ms` milliseconds since start.
    ///
    /// Fails closed when the evidence is for another instance or epoch
    /// ([`ReadinessWaitError::ForeignReadiness`]) or a superseded generation
    /// ([`ReadinessWaitError::StaleReadiness`]); such an observation never
    /// consumes the attempt budget. Exhausting the deadline or attempt budget
    /// times the wait out. An admitted observation below the target returns
    /// [`ReadinessProgress::AwaitingMore`] without settling; one that reaches the
    /// target settles the wait and returns [`ReadinessProgress::Satisfied`].
    pub fn observe<E: GameplayReadinessEvidence>(
        &mut self,
        evidence: &E,
        milestone: ReadinessMilestone,
        generation: u64,
        now_ms: u64,
    ) -> Result<ReadinessProgress, ReadinessWaitError> {
        if self.terminal.is_some() {
            return Err(ReadinessWaitError::Settled);
        }
        if evidence.instance_id() != self.instance_id
            || evidence.authority_epoch() != self.authority_epoch
        {
            return Err(ReadinessWaitError::ForeignReadiness);
        }
        if generation != self.generation {
            return Err(ReadinessWaitError::StaleReadiness);
        }
        if now_ms > self.target.deadline_ms {
            self.terminal = Some(ReadinessTerminal::TimedOut);
            return Err(ReadinessWaitError::Timeout);
        }
        self.attempts += 1;
        if self.attempts > self.target.max_attempts {
            self.terminal = Some(ReadinessTerminal::TimedOut);
            return Err(ReadinessWaitError::Timeout);
        }
        if milestone.reaches(self.target.milestone) {
            self.terminal = Some(ReadinessTerminal::Satisfied);
            Ok(ReadinessProgress::Satisfied)
        } else {
            Ok(ReadinessProgress::AwaitingMore)
        }
    }

    /// Records that the owning lifecycle surface denied readiness.
    pub fn deny(&mut self) -> Result<(), ReadinessWaitError> {
        self.settle(ReadinessTerminal::Denied)
    }

    /// Records that the workflow cancelled the wait.
    pub fn cancel(&mut self) -> Result<(), ReadinessWaitError> {
        self.settle(ReadinessTerminal::Cancelled)
    }

    /// Records that a restart invalidated prior readiness for this generation.
    ///
    /// The wait must not be reused after a restart: a fresh wait bound to the new
    /// generation is required.
    pub fn invalidate_for_restart(&mut self) -> Result<(), ReadinessWaitError> {
        self.settle(ReadinessTerminal::Invalidated)
    }

    fn settle(&mut self, terminal: ReadinessTerminal) -> Result<(), ReadinessWaitError> {
        if self.terminal.is_some() {
            return Err(ReadinessWaitError::Settled);
        }
        self.terminal = Some(terminal);
        Ok(())
    }
}
