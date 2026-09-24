// SPDX-License-Identifier: MIT

//! A resumable, retry-safe scheduler over one immutable branch-experiment plan.
//!
//! The scheduler keeps attempt lineage: a retry increments the attempt count, a replayed settlement
//! is idempotent, and a conflicting settlement is refused rather than double-scored. Starting a
//! settled or cancelled trial is refused, so a trial can never be continued as if it were a fresh
//! start, and cancelling settles no result.

use std::collections::BTreeMap;

use super::declaration::BranchExperimentManifest;
use super::error::BranchExperimentError;
use super::outcome::BranchOutcome;
use super::plan::{PlannedBranchTrial, plan};

/// The position of one planned trial within the scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrialPhase {
    /// Not started yet.
    Pending,
    /// Started at least once and not yet settled.
    Running,
    /// Settled with a recorded outcome; never re-scored.
    Settled,
    /// Cancelled without a recorded result.
    Cancelled,
}

/// Result of one settlement attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Settlement {
    /// The outcome was recorded for the first time.
    Recorded,
    /// The identical outcome was already recorded; nothing changed.
    Duplicate,
}

/// Rejection reasons for a scheduling transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleError {
    /// The key is not part of the declared plan.
    UnknownTrial(String),
    /// The trial already settled; its outcome is never erased or replaced.
    AlreadyScored(String),
    /// The trial was cancelled; no result may be recorded or restarted for it.
    AlreadyCancelled(String),
    /// A different outcome already exists for this trial key.
    ConflictingOutcome(String),
    /// The supplied outcome is malformed.
    InvalidOutcome(BranchExperimentError),
    /// The declaration was rejected.
    Declaration(BranchExperimentError),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::UnknownTrial(_) => "trial is not part of the plan",
            Self::AlreadyScored(_) => "trial is already settled",
            Self::AlreadyCancelled(_) => "trial is cancelled",
            Self::ConflictingOutcome(_) => "trial already has a different outcome",
            Self::InvalidOutcome(_) => "trial outcome is invalid",
            Self::Declaration(_) => "branch experiment declaration is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ScheduleError {}

impl From<BranchExperimentError> for ScheduleError {
    fn from(error: BranchExperimentError) -> Self {
        Self::Declaration(error)
    }
}

struct TrialSlot {
    planned: PlannedBranchTrial,
    phase: TrialPhase,
    attempts: u32,
    outcome: Option<BranchOutcome>,
}

/// A resumable, retry-safe view over one immutable branch-experiment plan.
pub struct BranchExperimentScheduler {
    manifest: BranchExperimentManifest,
    revision: String,
    slots: Vec<TrialSlot>,
    index: BTreeMap<String, usize>,
}

impl BranchExperimentScheduler {
    /// Builds a scheduler over a validated experiment declaration.
    ///
    /// # Errors
    ///
    /// Returns the declaration rejection for an invalid declaration.
    pub fn new(manifest: &BranchExperimentManifest) -> Result<Self, ScheduleError> {
        manifest.validate().map_err(ScheduleError::Declaration)?;
        let planned = plan(manifest).map_err(ScheduleError::Declaration)?;
        let revision = manifest.digest().map_err(ScheduleError::Declaration)?;
        let mut index = BTreeMap::new();
        let slots = planned
            .into_iter()
            .enumerate()
            .map(|(position, planned)| {
                index.insert(planned.trial_key.clone(), position);
                TrialSlot {
                    planned,
                    phase: TrialPhase::Pending,
                    attempts: 0,
                    outcome: None,
                }
            })
            .collect();
        Ok(Self {
            manifest: manifest.clone(),
            revision,
            slots,
            index,
        })
    }

    /// Restores a scheduler and replays every previously recorded outcome.
    ///
    /// # Errors
    ///
    /// Returns the declaration rejection, or [`ScheduleError`] when a supplied outcome is
    /// off-plan, malformed or conflicts with an equal-key outcome.
    pub fn resume(
        manifest: &BranchExperimentManifest,
        outcomes: impl IntoIterator<Item = BranchOutcome>,
    ) -> Result<Self, ScheduleError> {
        let mut scheduler = Self::new(manifest)?;
        for outcome in outcomes {
            scheduler.settle(outcome)?;
        }
        Ok(scheduler)
    }

    /// Returns the immutable declaration this scheduler was built from.
    #[must_use]
    pub fn manifest(&self) -> &BranchExperimentManifest {
        &self.manifest
    }

    /// Returns the experiment revision identity of the plan.
    #[must_use]
    pub fn experiment_revision(&self) -> &str {
        &self.revision
    }

    /// Returns every planned trial in declaration order.
    #[must_use]
    pub fn plan(&self) -> Vec<&PlannedBranchTrial> {
        self.slots.iter().map(|slot| &slot.planned).collect()
    }

    /// Returns the number of planned trials.
    #[must_use]
    pub fn planned_count(&self) -> usize {
        self.slots.len()
    }

    /// Returns every not-yet-settled, not-yet-cancelled trial in declaration order.
    #[must_use]
    pub fn pending(&self) -> Vec<&PlannedBranchTrial> {
        self.slots
            .iter()
            .filter(|slot| matches!(slot.phase, TrialPhase::Pending | TrialPhase::Running))
            .map(|slot| &slot.planned)
            .collect()
    }

    /// Returns the current phase of a planned trial.
    #[must_use]
    pub fn phase_of(&self, key: &str) -> Option<TrialPhase> {
        self.slot(key).map(|slot| slot.phase)
    }

    /// Returns the recorded attempt count of a planned trial.
    #[must_use]
    pub fn attempts(&self, key: &str) -> Option<u32> {
        self.slot(key).map(|slot| slot.attempts)
    }

    /// Starts a trial or records a retry; returns the new attempt count.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleError`] for an unknown, settled or cancelled trial.
    pub fn start(&mut self, key: &str) -> Result<u32, ScheduleError> {
        let slot = self.slot_mut(key)?;
        match slot.phase {
            TrialPhase::Settled => return Err(ScheduleError::AlreadyScored(key.to_owned())),
            TrialPhase::Cancelled => return Err(ScheduleError::AlreadyCancelled(key.to_owned())),
            TrialPhase::Pending | TrialPhase::Running => {}
        }
        slot.attempts = slot.attempts.saturating_add(1);
        slot.phase = TrialPhase::Running;
        Ok(slot.attempts)
    }

    /// Cancels a trial without scoring it; cancelling twice is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleError::AlreadyScored`] for a settled trial and
    /// [`ScheduleError::UnknownTrial`] for an unknown key.
    pub fn cancel(&mut self, key: &str) -> Result<(), ScheduleError> {
        let slot = self.slot_mut(key)?;
        match slot.phase {
            TrialPhase::Settled => Err(ScheduleError::AlreadyScored(key.to_owned())),
            TrialPhase::Cancelled => Ok(()),
            TrialPhase::Pending | TrialPhase::Running => {
                slot.phase = TrialPhase::Cancelled;
                Ok(())
            }
        }
    }

    /// Records one outcome for a planned trial.
    ///
    /// A repeated identical outcome is an idempotent [`Settlement::Duplicate`]; a different outcome
    /// for an already-settled key is refused so a trial is never double-scored.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleError`] for an unknown key, an invalid outcome, a cancelled trial, or a
    /// conflicting outcome on an already-settled trial.
    pub fn settle(&mut self, outcome: BranchOutcome) -> Result<Settlement, ScheduleError> {
        outcome.validate().map_err(ScheduleError::InvalidOutcome)?;
        let key = outcome.trial_key.clone();
        let slot = self.slot_mut(&key)?;
        match (slot.phase, &slot.outcome) {
            (TrialPhase::Settled, Some(existing)) if existing == &outcome => {
                Ok(Settlement::Duplicate)
            }
            (TrialPhase::Settled, _) => Err(ScheduleError::ConflictingOutcome(key)),
            (TrialPhase::Cancelled, _) => Err(ScheduleError::AlreadyCancelled(key)),
            (TrialPhase::Pending | TrialPhase::Running, _) => {
                slot.attempts = slot.attempts.max(outcome.attempts);
                slot.outcome = Some(outcome);
                slot.phase = TrialPhase::Settled;
                Ok(Settlement::Recorded)
            }
        }
    }

    /// Returns every recorded outcome in declaration order.
    #[must_use]
    pub fn outcomes(&self) -> Vec<&BranchOutcome> {
        self.slots
            .iter()
            .filter_map(|slot| slot.outcome.as_ref())
            .collect()
    }

    /// Reports whether every planned trial is settled or cancelled.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.slots
            .iter()
            .all(|slot| matches!(slot.phase, TrialPhase::Settled | TrialPhase::Cancelled))
    }

    fn slot(&self, key: &str) -> Option<&TrialSlot> {
        self.index
            .get(key)
            .and_then(|position| self.slots.get(*position))
    }

    fn slot_mut(&mut self, key: &str) -> Result<&mut TrialSlot, ScheduleError> {
        let position = self
            .index
            .get(key)
            .copied()
            .ok_or_else(|| ScheduleError::UnknownTrial(key.to_owned()))?;
        self.slots
            .get_mut(position)
            .ok_or_else(|| ScheduleError::UnknownTrial(key.to_owned()))
    }
}
