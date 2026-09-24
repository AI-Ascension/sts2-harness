// SPDX-License-Identifier: MIT

//! Bounded scheduling and reporting rejection vocabularies.

use std::fmt;

use serde::Serialize;

use super::results::TrialOutcomeError;

/// Rejection reasons for a suite manifest or the work it declares.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteManifestError {
    /// The declared version is not [`SUITE_VERSION`](super::manifest::SUITE_VERSION).
    UnsupportedVersion,
    /// The benchmark reference is empty, oversized or contains a NUL separator.
    InvalidBenchmarkRef,
    /// The evaluator revision is empty, oversized or contains a NUL separator.
    InvalidEvaluatorRevision,
    /// A case, policy, metric or settings label is empty, oversized or contains a NUL separator.
    InvalidLabel,
    /// The corpus declares no case.
    EmptyCorpus,
    /// The corpus exceeds [`MAX_SUITE_CASES`](super::manifest::MAX_SUITE_CASES).
    TooManyCases,
    /// Two cases share a `case_id`.
    DuplicateCase,
    /// The suite declares no policy.
    EmptyPolicies,
    /// The suite exceeds [`MAX_SUITE_POLICIES`](super::manifest::MAX_SUITE_POLICIES).
    TooManyPolicies,
    /// Two policies share a `policy_id`.
    DuplicatePolicy,
    /// A case id and a policy id pair would derive a trial key beyond
    /// [`MAX_TRIAL_KEY_BYTES`](super::results::MAX_TRIAL_KEY_BYTES).
    TrialKeyOverflow,
    /// A policy settings digest is empty, oversized or contains a NUL separator.
    InvalidSettingsDigest,
    /// The repetition count is zero or exceeds
    /// [`MAX_SUITE_REPETITIONS`](super::manifest::MAX_SUITE_REPETITIONS).
    InvalidRepetitions,
    /// The suite declares no metric.
    EmptyMetrics,
    /// The suite exceeds [`MAX_SUITE_METRICS`](super::manifest::MAX_SUITE_METRICS).
    TooManyMetrics,
    /// Two metrics share a name.
    DuplicateMetric,
    /// A budget bound is zero, or the concurrency bound is outside
    /// `1..=MAX_SUITE_CONCURRENCY` ([`MAX_SUITE_CONCURRENCY`](super::manifest::MAX_SUITE_CONCURRENCY)).
    InvalidBudget,
    /// The declared axes would overflow the planned trial count.
    PlanOverflow,
    /// The canonical encoding exceeded
    /// [`MAX_SUITE_MANIFEST_BYTES`](super::manifest::MAX_SUITE_MANIFEST_BYTES).
    TooLarge,
    /// The manifest could not be encoded canonically.
    NotEncodable,
}

impl fmt::Display for SuiteManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedVersion => "unsupported suite version",
            Self::InvalidBenchmarkRef => "invalid benchmark reference",
            Self::InvalidEvaluatorRevision => "invalid evaluator revision",
            Self::InvalidLabel => "invalid label",
            Self::EmptyCorpus => "empty seed corpus",
            Self::TooManyCases => "too many seed cases",
            Self::DuplicateCase => "duplicate seed case",
            Self::EmptyPolicies => "empty policy axis",
            Self::TooManyPolicies => "too many policies",
            Self::DuplicatePolicy => "duplicate policy",
            Self::TrialKeyOverflow => "case and policy axes overflow the trial key bound",
            Self::InvalidSettingsDigest => "invalid settings digest",
            Self::InvalidRepetitions => "invalid repetition count",
            Self::EmptyMetrics => "empty metric set",
            Self::TooManyMetrics => "too many metrics",
            Self::DuplicateMetric => "duplicate metric",
            Self::InvalidBudget => "invalid budget",
            Self::PlanOverflow => "planned trial count overflow",
            Self::TooLarge => "suite manifest exceeds the byte bound",
            Self::NotEncodable => "suite manifest is not encodable",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SuiteManifestError {}

/// Rejection reasons for a scheduling transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleError {
    /// The key is not part of the frozen plan.
    UnknownTrial(String),
    /// The trial already settled; its outcome is never erased or replaced.
    AlreadyScored(String),
    /// The trial was cancelled; no result may be recorded for it.
    AlreadyCancelled(String),
    /// A different outcome already exists for this trial key.
    ConflictingOutcome(String),
    /// The supplied outcome is malformed.
    InvalidOutcome(TrialOutcomeError),
    /// The suite declaration was rejected.
    Manifest(SuiteManifestError),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnknownTrial(_) => "trial is not part of the plan",
            Self::AlreadyScored(_) => "trial is already settled",
            Self::AlreadyCancelled(_) => "trial is cancelled",
            Self::ConflictingOutcome(_) => "trial already has a different outcome",
            Self::InvalidOutcome(_) => "trial outcome is invalid",
            Self::Manifest(_) => "suite manifest is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ScheduleError {}

impl From<TrialOutcomeError> for ScheduleError {
    fn from(error: TrialOutcomeError) -> Self {
        Self::InvalidOutcome(error)
    }
}

impl From<SuiteManifestError> for ScheduleError {
    fn from(error: SuiteManifestError) -> Self {
        Self::Manifest(error)
    }
}

/// Rejection reasons for a suite aggregate, comparison or witness audit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportError {
    /// The suite declaration was rejected.
    Manifest(SuiteManifestError),
    /// A recorded outcome is malformed.
    InvalidOutcome(TrialOutcomeError),
    /// A predeclared metric label is not a known metric.
    UnknownMetric(String),
    /// A recorded outcome has no planned trial.
    UnplannedOutcome(String),
    /// Two recorded outcomes share one trial key.
    DuplicateOutcome(String),
    /// A requested policy is not part of the suite.
    UnknownPolicy(String),
    /// The two compared policies are the same.
    RepeatedPolicy(String),
    /// Two verified initial witnesses disagree for one seed case.
    WitnessMismatch(String),
    /// The sanitized report could not be encoded.
    NotEncodable,
}

impl fmt::Display for ReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Manifest(_) => "suite manifest is invalid",
            Self::InvalidOutcome(_) => "trial outcome is invalid",
            Self::UnknownMetric(_) => "predeclared metric is unknown",
            Self::UnplannedOutcome(_) => "outcome has no planned trial",
            Self::DuplicateOutcome(_) => "trial has more than one outcome",
            Self::UnknownPolicy(_) => "policy is not part of the suite",
            Self::RepeatedPolicy(_) => "compared policies are identical",
            Self::WitnessMismatch(_) => "initial native witnesses disagree",
            Self::NotEncodable => "suite report could not be encoded",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ReportError {}

impl From<TrialOutcomeError> for ReportError {
    fn from(error: TrialOutcomeError) -> Self {
        Self::InvalidOutcome(error)
    }
}

impl From<SuiteManifestError> for ReportError {
    fn from(error: SuiteManifestError) -> Self {
        Self::Manifest(error)
    }
}
