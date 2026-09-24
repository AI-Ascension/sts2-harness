// SPDX-License-Identifier: MIT

//! Per-trial branch outcomes with retained settled transitions and honest classification.
//!
//! One trial settles at most one outcome. A cancelled or budget-censored trial keeps whatever
//! partial evidence exists but is never counted as a defeat; an unmeasured quantity stays
//! explicitly unavailable rather than zero; and a restore failure is recorded as its own status,
//! never silently folded into a policy result.

use crate::{ExactAssurance, TransitionTrace};

use super::admission::is_verified_start;
use super::error::BranchExperimentError;
use super::label::label_ok;

use crate::benchmark_manifest::suite::Measurement;

/// Terminal classification of one logical branch trial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchStatus {
    /// The trial settled a declared terminal game result.
    Completed,
    /// A declared budget bound stopped the trial before it settled.
    BudgetCensored,
    /// The trial was cancelled; no result is scored.
    Cancelled,
    /// The environment failed before any game outcome existed.
    InfrastructureFailure,
    /// The trial ended without a decidable outcome.
    UnknownOutcome,
    /// The start or a replay failed to restore the shared exact state; this is not a game result.
    RestoreFailed,
}

impl BranchStatus {
    /// Returns the stable lowercase label of this status.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::BudgetCensored => "budget_censored",
            Self::Cancelled => "cancelled",
            Self::InfrastructureFailure => "infrastructure_failure",
            Self::UnknownOutcome => "unknown_outcome",
            Self::RestoreFailed => "restore_failed",
        }
    }

    /// Reports whether this status carries a settled game result.
    #[must_use]
    pub fn is_decided(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// The recorded outcome of exactly one logical branch trial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchOutcome {
    /// Stable plan key of the trial.
    pub trial_key: String,
    /// Terminal classification.
    pub status: BranchStatus,
    /// Attempts consumed before this outcome was observed; always at least one.
    pub attempts: u32,
    /// Verified start assurance recorded for this trial, when one was admitted.
    pub start_assurance: Option<ExactAssurance>,
    /// Settled transition records retained from the live owner path.
    pub trace: Option<TransitionTrace>,
    /// Highest floor the trial reached when measured.
    pub reached_floor: Measurement<u32>,
    /// Settled decision count when measured.
    pub decision_count: Measurement<u64>,
    /// Measured wall-clock latency when available.
    pub latency_millis: Measurement<u64>,
    /// Measured provider tokens when available.
    pub provider_tokens: Measurement<u64>,
    /// Measured provider spend in micros when available; never assumed zero.
    pub cost_micros: Measurement<u64>,
}

impl BranchOutcome {
    fn blank(trial_key: &str, status: BranchStatus) -> Self {
        Self {
            trial_key: trial_key.to_owned(),
            status,
            attempts: 1,
            start_assurance: None,
            trace: None,
            reached_floor: Measurement::Unavailable,
            decision_count: Measurement::Unavailable,
            latency_millis: Measurement::Unavailable,
            provider_tokens: Measurement::Unavailable,
            cost_micros: Measurement::Unavailable,
        }
    }

    /// Records a trial that settled with retained transition evidence.
    #[must_use]
    pub fn completed(trial_key: &str, trace: TransitionTrace) -> Self {
        Self::blank(trial_key, BranchStatus::Completed).with_trace(trace)
    }

    /// Records a trial stopped by a declared budget bound.
    #[must_use]
    pub fn budget_censored(trial_key: &str) -> Self {
        Self::blank(trial_key, BranchStatus::BudgetCensored)
    }

    /// Records a cancelled trial.
    #[must_use]
    pub fn cancelled(trial_key: &str) -> Self {
        Self::blank(trial_key, BranchStatus::Cancelled)
    }

    /// Records an infrastructure failure; no game result exists.
    #[must_use]
    pub fn infrastructure_failure(trial_key: &str) -> Self {
        Self::blank(trial_key, BranchStatus::InfrastructureFailure)
    }

    /// Records a trial whose outcome stayed unknown.
    #[must_use]
    pub fn unknown_outcome(trial_key: &str) -> Self {
        Self::blank(trial_key, BranchStatus::UnknownOutcome)
    }

    /// Records a restore failure; this is an invalid comparison, not a defeat.
    #[must_use]
    pub fn restore_failed(trial_key: &str) -> Self {
        Self::blank(trial_key, BranchStatus::RestoreFailed)
    }

    /// Attaches the verified assurance of the admitted start.
    #[must_use]
    pub fn with_start(mut self, assurance: ExactAssurance) -> Self {
        self.start_assurance = Some(assurance);
        self
    }

    /// Attaches retained settled transition evidence.
    #[must_use]
    pub fn with_trace(mut self, trace: TransitionTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Validates the key, attempt count, status and retained evidence.
    ///
    /// # Errors
    ///
    /// Returns [`BranchExperimentError::InvalidOutcome`] for any inconsistency.
    pub fn validate(&self) -> Result<(), BranchExperimentError> {
        if !label_ok(&self.trial_key) || self.attempts == 0 {
            return Err(BranchExperimentError::InvalidOutcome);
        }
        if self.status == BranchStatus::Completed && self.trace.is_none() {
            return Err(BranchExperimentError::InvalidOutcome);
        }
        if self.status == BranchStatus::RestoreFailed && self.trace.is_some() {
            return Err(BranchExperimentError::InvalidOutcome);
        }
        if let Some(trace) = &self.trace {
            trace
                .validate()
                .map_err(|_| BranchExperimentError::InvalidOutcome)?;
        }
        Ok(())
    }

    /// Reports whether this outcome may enter exact-restore statistics.
    ///
    /// A prefix-only start, a cancelled or unknown trial, an infrastructure failure and a restore
    /// failure are all excluded, so an unverified start never blends into a certified statistic.
    #[must_use]
    pub fn exact_restore_eligible(&self) -> bool {
        matches!(
            self.status,
            BranchStatus::Completed | BranchStatus::BudgetCensored
        ) && self.trace.is_some()
            && self.start_assurance.is_some_and(is_verified_start)
    }
}
