// SPDX-License-Identifier: MIT

//! Bounded rejection vocabularies for declaration, admission and comparison.

use std::fmt;

/// Rejection reasons for a branch-experiment declaration, plan or outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchExperimentError {
    /// The declaration version is not supported.
    UnsupportedVersion,
    /// A label is empty, too long, or contains a NUL separator.
    InvalidLabel,
    /// The shared fork-point reference is invalid.
    InvalidForkPoint,
    /// The experiment declares no child.
    EmptyChildren,
    /// The experiment declares more than the supported child bound.
    TooManyChildren,
    /// A child label repeats another child.
    DuplicateChild,
    /// The strategy requires an explicit first action the child does not declare.
    AlternativeFirstActionRequired,
    /// The strategy forbids an explicit first action the child declares.
    UnexpectedFirstAction,
    /// A child settings digest is not `sha256:` plus lowercase hex.
    InvalidSettingsDigest,
    /// A declared budget is zero, inverted, or out of range.
    InvalidBudget,
    /// The experiment declares no stop condition.
    EmptyStopConditions,
    /// A recorded outcome is malformed or internally inconsistent.
    InvalidOutcome,
    /// The declaration exceeds its whole-input bound.
    TooLarge,
}

impl fmt::Display for BranchExperimentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedVersion => "branch-experiment version is not supported",
            Self::InvalidLabel => "branch-experiment label is invalid",
            Self::InvalidForkPoint => "branch-experiment fork point is invalid",
            Self::EmptyChildren => "branch-experiment declares no child",
            Self::TooManyChildren => "branch-experiment declares too many children",
            Self::DuplicateChild => "branch-experiment repeats a child label",
            Self::AlternativeFirstActionRequired => "child must declare a first action",
            Self::UnexpectedFirstAction => "child must not declare a first action",
            Self::InvalidSettingsDigest => "child settings digest is invalid",
            Self::InvalidBudget => "branch-experiment budget is invalid",
            Self::EmptyStopConditions => "branch-experiment declares no stop condition",
            Self::InvalidOutcome => "branch trial outcome is invalid",
            Self::TooLarge => "branch experiment exceeds its bound",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for BranchExperimentError {}

/// Rejection reasons for same-start admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    /// The trial is not part of the declared plan.
    UnknownTrial(String),
    /// The observed start reference is itself malformed.
    InvalidReference,
    /// The observed start is a different exact checkpoint than the shared fork point.
    StartMismatch,
    /// The observed start does not carry a verified restore.
    StartNotVerified,
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnknownTrial(_) => "trial is not part of the declared plan",
            Self::InvalidReference => "start reference is invalid",
            Self::StartMismatch => "start is not the shared verified fork point",
            Self::StartNotVerified => "start does not carry a verified restore",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for AdmissionError {}

/// Rejection reasons for a branch comparison or aggregate report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComparisonError {
    /// A requested child is not part of the experiment.
    UnknownChild(String),
    /// The two compared children are the same.
    RepeatedChild(String),
    /// A recorded outcome is malformed.
    InvalidOutcome(BranchExperimentError),
    /// The experiment declaration was rejected while comparing.
    Declaration(BranchExperimentError),
    /// A recorded outcome has no planned trial.
    UnplannedOutcome(String),
    /// Two recorded outcomes share one trial key.
    DuplicateOutcome(String),
    /// The projection key is unusable for a public export.
    InvalidKey,
    /// The sanitized report could not be encoded.
    NotEncodable,
}

impl fmt::Display for ComparisonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnknownChild(_) => "child is not part of the experiment",
            Self::RepeatedChild(_) => "compared children are identical",
            Self::InvalidOutcome(_) => "branch trial outcome is invalid",
            Self::Declaration(_) => "branch experiment declaration is invalid",
            Self::UnplannedOutcome(_) => "outcome has no planned trial",
            Self::DuplicateOutcome(_) => "trial has more than one outcome",
            Self::InvalidKey => "public projection key is unusable",
            Self::NotEncodable => "branch experiment report could not be encoded",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ComparisonError {}
