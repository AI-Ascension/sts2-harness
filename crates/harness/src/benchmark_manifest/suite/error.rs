// SPDX-License-Identifier: MIT

//! Bounded scheduling and reporting rejection vocabularies.

use std::fmt;

use super::manifest::SuiteManifestError;
use super::results::TrialOutcomeError;

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
