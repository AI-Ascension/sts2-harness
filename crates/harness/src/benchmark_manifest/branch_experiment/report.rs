// SPDX-License-Identifier: MIT

//! Sanitized comparison and aggregate-report vocabulary for one experiment.
//!
//! These types are the only public/model payloads of a branch experiment. They carry a keyed
//! handle and categorical labels, never an exact game digest, checkpoint id or action key.

use serde::{Deserialize, Serialize};

use super::error::ComparisonError;

/// Honest classification of the first divergence between two child branches.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchDivergence {
    /// Every recorded and observed boundary matched.
    IdenticalOverRecordedRange,
    /// The two children deliberately chose different actions under different declared policies.
    PolicyDivergence,
    /// The two children chose different actions despite the same declared policy.
    DifferentAction,
    /// The controlled external input differed.
    DifferentExternalInput,
    /// Exact state differed with matching actions and inputs.
    StateDivergence,
    /// A required capture or catalog witness, or a whole trace, is absent.
    MissingCapture,
    /// A start failed to restore the shared exact state; this is not a policy result.
    RestoreFailure,
    /// The traces were recorded under incomparable profiles.
    IncompatibleTrace,
    /// The boundary coordinates do not align.
    UnalignedTrace,
    /// One trace contains no recorded boundary.
    InsufficientCoverage,
}

impl BranchDivergence {
    /// Returns the stable lowercase label of this divergence.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::IdenticalOverRecordedRange => "identical_over_recorded_range",
            Self::PolicyDivergence => "policy_divergence",
            Self::DifferentAction => "different_action",
            Self::DifferentExternalInput => "different_external_input",
            Self::StateDivergence => "state_divergence",
            Self::MissingCapture => "missing_capture",
            Self::RestoreFailure => "restore_failure",
            Self::IncompatibleTrace => "incompatible_trace",
            Self::UnalignedTrace => "unaligned_trace",
            Self::InsufficientCoverage => "insufficient_coverage",
        }
    }
}

/// One bounded comparison of two child branches of one experiment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BranchComparison {
    /// First child compared.
    pub child_a: String,
    /// Second child compared.
    pub child_b: String,
    /// Classification of the first divergence.
    pub divergence: BranchDivergence,
    /// Whether both sides may enter exact-restore statistics.
    pub exact_restore: bool,
    /// Ordinal of the first observed divergence, when one exists.
    pub first_divergence_ordinal: Option<u64>,
    /// Ordinal of the last verified equal boundary.
    pub last_equal_ordinal: Option<u64>,
    /// Number of aligned boundaries examined.
    pub compared_actions: u64,
    /// Boundaries present in one trace but not the other.
    ///
    /// A branch comparison has no authoritative side, so this count is symmetric: it reports how
    /// many recorded boundaries one child has that the other does not, independent of argument
    /// order. It is zero whenever the two traces diverge over their shared range.
    #[serde(default)]
    pub unobserved_records: u64,
}

impl BranchComparison {
    /// Reports whether the first divergence is declared policy divergence.
    #[must_use]
    pub fn is_policy_divergence(&self) -> bool {
        matches!(self.divergence, BranchDivergence::PolicyDivergence)
    }
}

/// The sanitized aggregate report for one experiment; it carries no exact digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BranchExperimentReport {
    /// Public report envelope version.
    pub version: &'static str,
    /// Keyed opaque experiment handle, never a raw digest.
    pub experiment_ref: String,
    /// Declared fork strategy label.
    pub strategy: &'static str,
    /// Planned logical trials.
    pub planned: u64,
    /// Trials that settled a terminal result.
    pub settled: u64,
    /// Trials stopped by a declared budget bound.
    pub censored: u64,
    /// Cancelled trials.
    pub cancelled: u64,
    /// Infrastructure failures and unknown outcomes.
    pub unknown: u64,
    /// Restore failures, distinct from any game result.
    pub restore_failures: u64,
    /// Trials eligible for exact-restore statistics.
    pub exact_restore_settled: u64,
    /// Every unordered pair comparison.
    pub comparisons: Vec<BranchComparison>,
}

impl BranchExperimentReport {
    /// Encodes the sanitized report as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ComparisonError::NotEncodable`] when the report cannot be encoded.
    pub fn to_json_pretty(&self) -> Result<String, ComparisonError> {
        serde_json::to_string_pretty(self).map_err(|_| ComparisonError::NotEncodable)
    }
}
