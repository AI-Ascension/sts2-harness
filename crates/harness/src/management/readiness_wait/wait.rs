// SPDX-License-Identifier: MIT

//! A per-generation bounded readiness wait.
//!
//! A wait is started for one instance, authority epoch and process generation.
//! It admits observations through a bounded deadline and attempt budget, and it
//! settles only from an observation that carries that same instance and epoch,
//! states the current generation, and reports a milestone that reaches the
//! target. A foreign, stale or below-target observation cannot settle it, and a
//! restart invalidates prior readiness so fresh proof is required.
//!
//! Everything that decides settlement travels in one owner-constructed
//! [`MilestoneObservation`]: the sealed instance/epoch proof and the reported
//! milestone and generation. The wait takes no separate milestone or generation
//! argument, so one owner's proof cannot be attributed to another milestone.
//!
//! The wait owns no clock. `now_ms` is the caller's time in the target's units,
//! and the bounded deadline takes effect only when the caller advances the
//! wait: an admitted observation is refused once the deadline has elapsed, and
//! [`ReadinessWait::expire_if_elapsed`] times out a wait that never receives
//! one, so a starved wait cannot stay open forever.

use crate::management::{GameplayReadinessEvidence, ReadinessObservation};

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

/// The outcome of advancing a wait's own bounded clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessExpiry {
    /// The bounded deadline has not elapsed; the wait is still open.
    StillOpen,
    /// The bounded deadline elapsed and this call timed the wait out.
    TimedOut,
    /// The wait had already settled; its terminal outcome is unchanged.
    AlreadySettled(ReadinessTerminal),
}

/// Evidence that an owner observed one readiness milestone under one process
/// generation.
///
/// The sealed instance/authority-epoch proof ([`ReadinessObservation`]) and the
/// owner-reported milestone and generation travel in a single value, so the
/// facts that decide settlement cannot be mixed and matched across
/// observations. Construction requires the sealed owner observation, so a
/// launch acknowledgement still cannot become readiness evidence, and the
/// observation identity that a record can re-check is carried alongside.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneObservation {
    readiness: ReadinessObservation,
    milestone: ReadinessMilestone,
    generation: u64,
}

impl MilestoneObservation {
    /// Binds an owner-reported milestone and generation to its readiness proof.
    ///
    /// Refuses a zero generation ([`ReadinessWaitError::InvalidBinding`]): a
    /// milestone observed under no generation cannot be checked against a
    /// wait's binding, so it is not an observation this contract accepts.
    pub fn new(
        readiness: ReadinessObservation,
        milestone: ReadinessMilestone,
        generation: u64,
    ) -> Result<Self, ReadinessWaitError> {
        if generation == 0 {
            return Err(ReadinessWaitError::InvalidBinding);
        }
        Ok(Self {
            readiness,
            milestone,
            generation,
        })
    }

    /// The instance the observation was made for.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        self.readiness.instance_id()
    }

    /// The authority epoch the observation was made under.
    #[must_use]
    pub fn authority_epoch(&self) -> u64 {
        self.readiness.authority_epoch()
    }

    /// The milestone the owner reported.
    #[must_use]
    pub const fn milestone(&self) -> ReadinessMilestone {
        self.milestone
    }

    /// The process generation the milestone was observed under.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The observation identity the owner read, for record cross-checking.
    #[must_use]
    pub fn observation_id(&self) -> &str {
        self.readiness.observation_id()
    }
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
    /// Fails closed when the observation carries another instance or epoch
    /// ([`ReadinessWaitError::ForeignReadiness`]) or a superseded generation
    /// ([`ReadinessWaitError::StaleReadiness`]); such an observation never
    /// consumes the attempt budget. Exhausting the deadline or attempt budget
    /// times the wait out. An admitted observation below the target returns
    /// [`ReadinessProgress::AwaitingMore`] without settling; one that reaches the
    /// target settles the wait and returns [`ReadinessProgress::Satisfied`].
    ///
    /// The milestone and generation are read from the observation, not from
    /// caller arguments, so the proof and the facts it settles cannot be
    /// supplied separately.
    pub fn observe(
        &mut self,
        observation: &MilestoneObservation,
        now_ms: u64,
    ) -> Result<ReadinessProgress, ReadinessWaitError> {
        if self.terminal.is_some() {
            return Err(ReadinessWaitError::Settled);
        }
        if observation.instance_id() != self.instance_id
            || observation.authority_epoch() != self.authority_epoch
        {
            return Err(ReadinessWaitError::ForeignReadiness);
        }
        if observation.generation() != self.generation {
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
        if observation.milestone().reaches(self.target.milestone) {
            self.terminal = Some(ReadinessTerminal::Satisfied);
            Ok(ReadinessProgress::Satisfied)
        } else {
            Ok(ReadinessProgress::AwaitingMore)
        }
    }

    /// Advances the wait's bounded clock without admitting an observation.
    ///
    /// `now_ms` is the caller's current time in the target's units. A wait that
    /// never receives an observation would otherwise stay open forever, because
    /// nothing else consults the clock, so a scheduler calls this while it is
    /// starved to let the bounded deadline take effect. A settled wait is
    /// reported unchanged and is never re-settled by a later call.
    pub fn expire_if_elapsed(&mut self, now_ms: u64) -> ReadinessExpiry {
        if let Some(terminal) = self.terminal {
            return ReadinessExpiry::AlreadySettled(terminal);
        }
        if now_ms > self.target.deadline_ms {
            self.terminal = Some(ReadinessTerminal::TimedOut);
            ReadinessExpiry::TimedOut
        } else {
            ReadinessExpiry::StillOpen
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
