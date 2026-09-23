// SPDX-License-Identifier: MIT

//! A resumable, retry-safe scheduler over one immutable suite plan.
//!
//! The scheduler keeps attempt lineage: a retry increments the attempt count, a replayed
//! settlement is idempotent, and a conflicting settlement is rejected rather than double-scored.
//! Cancelling settles no result, so a cancelled trial can never enter a win rate.

use std::collections::BTreeMap;

use super::error::ScheduleError;
use super::manifest::SuiteManifest;
use super::plan::{PlannedSuiteTrial, plan};
use super::results::TrialOutcome;

/// The position of one planned trial within the scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrialPhase {
    /// Not started yet.
    Pending,
    /// Started at least once and not yet settled.
    Running,
    /// Settled with a recorded outcome; never re-scored.
    Settled,
    /// Cancelled without a recorded outcome.
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

struct TrialSlot {
    planned: PlannedSuiteTrial,
    phase: TrialPhase,
    attempts: u32,
    outcome: Option<TrialOutcome>,
}

/// A resumable, retry-safe view over one immutable suite plan.
pub struct SuiteScheduler {
    manifest: SuiteManifest,
    suite_revision: String,
    slots: Vec<TrialSlot>,
    index: BTreeMap<String, usize>,
}

impl SuiteScheduler {
    /// Builds a scheduler over a validated suite plan.
    ///
    /// # Errors
    ///
    /// Returns the manifest rejection for an invalid declaration.
    pub fn new(manifest: &SuiteManifest) -> Result<Self, ScheduleError> {
        let planned = plan(manifest)?;
        let suite_revision = manifest.digest()?;
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
            suite_revision,
            slots,
            index,
        })
    }

    /// Restores a scheduler and replays every previously recorded outcome.
    ///
    /// # Errors
    ///
    /// Returns the manifest rejection for an invalid declaration, or [`ScheduleError`] when a
    /// supplied outcome is off-plan, malformed or conflicts with an equal-key outcome.
    pub fn resume(
        manifest: &SuiteManifest,
        outcomes: impl IntoIterator<Item = TrialOutcome>,
    ) -> Result<Self, ScheduleError> {
        let mut scheduler = Self::new(manifest)?;
        for outcome in outcomes {
            scheduler.settle(outcome)?;
        }
        Ok(scheduler)
    }

    /// Returns the immutable manifest this scheduler was built from.
    #[must_use]
    pub fn manifest(&self) -> &SuiteManifest {
        &self.manifest
    }

    /// Returns the suite revision identity of the plan.
    #[must_use]
    pub fn suite_revision(&self) -> &str {
        &self.suite_revision
    }

    /// Returns every planned trial in deterministic plan order.
    #[must_use]
    pub fn plan(&self) -> Vec<&PlannedSuiteTrial> {
        self.slots.iter().map(|slot| &slot.planned).collect()
    }

    /// Returns the number of planned trials.
    #[must_use]
    pub fn planned_count(&self) -> usize {
        self.slots.len()
    }

    /// Returns every not-yet-settled, not-yet-cancelled trial in plan order.
    #[must_use]
    pub fn pending(&self) -> Vec<&PlannedSuiteTrial> {
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
    /// A repeated identical outcome is an idempotent [`Settlement::Duplicate`]; a different
    /// outcome for an already-settled key is rejected so a trial is never double-scored.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduleError`] for an unknown key, an invalid outcome, a cancelled trial, or a
    /// conflicting outcome on an already-settled trial.
    pub fn settle(&mut self, outcome: TrialOutcome) -> Result<Settlement, ScheduleError> {
        outcome.validate()?;
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

    /// Returns every recorded outcome in deterministic plan order.
    #[must_use]
    pub fn outcomes(&self) -> Vec<&TrialOutcome> {
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
