// SPDX-License-Identifier: MIT

//! Machine-readable per-trial results and the metric vocabulary a suite predeclares.
//!
//! A trial settles at most one outcome. An unmeasured quantity stays explicitly
//! [`Measurement::Unavailable`]: it is never recorded as zero and never scored as a defeat. A
//! trial that failed in infrastructure carries no game result, so it cannot enter a win rate.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ExactCheckpointError, ExactStateDigest};

/// Maximum bytes of a stable trial key.
pub const MAX_TRIAL_KEY_BYTES: usize = 256;

/// Terminal classification of one logical suite trial.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialStatus {
    /// The trial reached and settled a declared game result.
    Completed,
    /// A declared budget bound stopped the trial before it settled.
    BudgetCensored,
    /// The trial was cancelled; no result is scored.
    Cancelled,
    /// The environment failed before any game outcome existed.
    InfrastructureFailure,
    /// The trial ended without a decidable outcome.
    UnknownOutcome,
}

impl TrialStatus {
    /// Returns the stable lowercase label of this status.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::BudgetCensored => "budget_censored",
            Self::Cancelled => "cancelled",
            Self::InfrastructureFailure => "infrastructure_failure",
            Self::UnknownOutcome => "unknown_outcome",
        }
    }

    /// Reports whether this status carries a settled win/loss result.
    #[must_use]
    pub fn is_decided(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// A settled game result; `Some` exactly on a [`TrialStatus::Completed`] trial.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialResult {
    /// The modelled policy won the episode.
    Victory,
    /// The modelled policy lost the episode.
    Defeat,
}

/// A measured quantity, or an explicit declaration that it was not measured.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "measurement", content = "value")]
pub enum Measurement<T> {
    /// The quantity was measured and carries this value.
    Measured(T),
    /// The quantity was not measured; it is never treated as zero.
    Unavailable,
}

impl<T> Measurement<T> {
    /// Reports whether a concrete value exists.
    #[must_use]
    pub fn is_measured(&self) -> bool {
        matches!(self, Self::Measured(_))
    }
}

impl<T: Copy> Measurement<T> {
    /// Returns the measured value, or `None` when it was not measured.
    #[must_use]
    pub fn value(&self) -> Option<T> {
        match self {
            Self::Measured(value) => Some(*value),
            Self::Unavailable => None,
        }
    }
}

/// A predeclared, nameable metric that every recorded trial carries a slot for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metric {
    /// Highest floor the trial reached.
    ReachedFloor,
    /// Settled action count.
    ActionCount,
    /// Measured wall-clock latency.
    LatencyMillis,
    /// Measured provider token usage.
    ProviderTokens,
    /// Measured provider spend in micros.
    CostMicros,
}

impl Metric {
    /// Every metric a suite may predeclare.
    pub const ALL: [Self; 5] = [
        Self::ReachedFloor,
        Self::ActionCount,
        Self::LatencyMillis,
        Self::ProviderTokens,
        Self::CostMicros,
    ];

    /// Returns the stable lowercase label of this metric.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ReachedFloor => "reached_floor",
            Self::ActionCount => "action_count",
            Self::LatencyMillis => "latency_millis",
            Self::ProviderTokens => "provider_tokens",
            Self::CostMicros => "cost_micros",
        }
    }

    /// Parses the stable label of a predeclared metric.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|metric| metric.label() == label)
    }

    /// Returns the measured value of this metric, or `None` when it was unavailable.
    #[must_use]
    pub fn measured_value(self, outcome: &TrialOutcome) -> Option<u64> {
        match self {
            Self::ReachedFloor => outcome.reached_floor.value().map(u64::from),
            Self::ActionCount => outcome.action_count.value(),
            Self::LatencyMillis => outcome.latency_millis.value(),
            Self::ProviderTokens => outcome.provider_tokens.value(),
            Self::CostMicros => outcome.cost_micros.value(),
        }
    }
}

/// A verified initial native witness, identified only by its exact-state digest.
///
/// Construction parses the `asc-state:v1:sha256:` namespace, so an arbitrary string cannot
/// masquerade as a verified start. The digest never enters a sanitized aggregate report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeWitness {
    digest: String,
}

impl NativeWitness {
    /// Parses an exact-state digest into a verified witness.
    ///
    /// # Errors
    ///
    /// Returns the parser error when `value` is not an exact-state digest.
    pub fn parse(value: &str) -> Result<Self, ExactCheckpointError> {
        let digest = ExactStateDigest::parse(value)?;
        Ok(Self {
            digest: digest.as_str().to_owned(),
        })
    }

    /// Returns the serialized exact-state digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.digest
    }
}

/// The recorded result of exactly one logical suite trial.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrialOutcome {
    /// Stable suite/case/policy/repetition key of the trial.
    pub trial_key: String,
    /// Terminal classification.
    pub status: TrialStatus,
    /// Attempts consumed before this outcome was observed; always at least one.
    pub attempts: u32,
    /// Settled game result; `Some` exactly when `status` is `Completed`.
    pub result: Option<TrialResult>,
    /// Highest floor the trial reached when measured.
    pub reached_floor: Measurement<u32>,
    /// Settled action count when measured.
    pub action_count: Measurement<u64>,
    /// Measured wall-clock latency when available.
    pub latency_millis: Measurement<u64>,
    /// Measured provider tokens when available.
    pub provider_tokens: Measurement<u64>,
    /// Measured provider spend in micros when available; never assumed zero.
    pub cost_micros: Measurement<u64>,
    /// Verified initial native witness; absent until a native capture certifies it.
    pub initial_witness: Option<NativeWitness>,
}

impl TrialOutcome {
    fn blank(trial_key: &str, status: TrialStatus, result: Option<TrialResult>) -> Self {
        Self {
            trial_key: trial_key.to_owned(),
            status,
            attempts: 1,
            result,
            reached_floor: Measurement::Unavailable,
            action_count: Measurement::Unavailable,
            latency_millis: Measurement::Unavailable,
            provider_tokens: Measurement::Unavailable,
            cost_micros: Measurement::Unavailable,
            initial_witness: None,
        }
    }

    /// Records a trial that settled a victory or a defeat.
    #[must_use]
    pub fn completed(trial_key: &str, result: TrialResult) -> Self {
        Self::blank(trial_key, TrialStatus::Completed, Some(result))
    }

    /// Records a trial stopped by a declared budget bound.
    #[must_use]
    pub fn budget_censored(trial_key: &str) -> Self {
        Self::blank(trial_key, TrialStatus::BudgetCensored, None)
    }

    /// Records a cancelled trial.
    #[must_use]
    pub fn cancelled(trial_key: &str) -> Self {
        Self::blank(trial_key, TrialStatus::Cancelled, None)
    }

    /// Records an infrastructure failure; no game result exists.
    #[must_use]
    pub fn infrastructure_failure(trial_key: &str) -> Self {
        Self::blank(trial_key, TrialStatus::InfrastructureFailure, None)
    }

    /// Records a trial whose outcome stayed unknown.
    #[must_use]
    pub fn unknown_outcome(trial_key: &str) -> Self {
        Self::blank(trial_key, TrialStatus::UnknownOutcome, None)
    }

    /// Records a measured metric value on this outcome.
    #[must_use]
    pub fn recording(mut self, metric: Metric, value: u64) -> Self {
        match metric {
            Metric::ReachedFloor => {
                self.reached_floor =
                    Measurement::Measured(u32::try_from(value).unwrap_or(u32::MAX));
            }
            Metric::ActionCount => self.action_count = Measurement::Measured(value),
            Metric::LatencyMillis => self.latency_millis = Measurement::Measured(value),
            Metric::ProviderTokens => self.provider_tokens = Measurement::Measured(value),
            Metric::CostMicros => self.cost_micros = Measurement::Measured(value),
        }
        self
    }

    /// Attaches a verified initial native witness.
    #[must_use]
    pub fn with_witness(mut self, witness: NativeWitness) -> Self {
        self.initial_witness = Some(witness);
        self
    }

    /// Records how many attempts the trial consumed before this outcome was observed.
    #[must_use]
    pub fn with_attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    /// Checks the status/result invariant and the trial-key bound.
    ///
    /// # Errors
    ///
    /// Returns [`TrialOutcomeError`] when the key is empty or oversized, when a completed trial
    /// carries no result, or when a non-completed trial carries one.
    pub fn validate(&self) -> Result<(), TrialOutcomeError> {
        if self.trial_key.is_empty() {
            return Err(TrialOutcomeError::EmptyTrialKey);
        }
        if self.trial_key.len() > MAX_TRIAL_KEY_BYTES {
            return Err(TrialOutcomeError::TrialKeyTooLong);
        }
        if self.attempts == 0 {
            return Err(TrialOutcomeError::ZeroAttempts);
        }
        match (self.status, self.result) {
            (TrialStatus::Completed, None) => Err(TrialOutcomeError::MissingResult),
            (TrialStatus::Completed, Some(_)) | (_, None) => Ok(()),
            (_, Some(_)) => Err(TrialOutcomeError::UnexpectedResult),
        }
    }
}

/// Rejection reasons for one recorded trial result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrialOutcomeError {
    /// The trial key is empty.
    EmptyTrialKey,
    /// The trial key exceeds [`MAX_TRIAL_KEY_BYTES`].
    TrialKeyTooLong,
    /// The recorded attempt count is zero.
    ZeroAttempts,
    /// A completed trial carries no victory/defeat result.
    MissingResult,
    /// A non-completed trial carries a victory/defeat result.
    UnexpectedResult,
}

impl fmt::Display for TrialOutcomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyTrialKey => "trial key is empty",
            Self::TrialKeyTooLong => "trial key exceeds the byte bound",
            Self::ZeroAttempts => "trial attempt count is zero",
            Self::MissingResult => "completed trial carries no result",
            Self::UnexpectedResult => "non-completed trial carries a result",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TrialOutcomeError {}
