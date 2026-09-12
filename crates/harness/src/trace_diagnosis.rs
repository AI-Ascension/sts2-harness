// SPDX-License-Identifier: MIT

//! Bounded divergence diagnosis with a privileged and a public face.
//!
//! Comparison produces a small structured report: the classification, the last verified equal
//! boundary, the first observed unequal boundary, and how many boundaries were examined or left
//! unobserved. Privileged evidence may also carry the exact state identities at the mismatch so an
//! evaluator can locate a restore diverging from a replay. The public status carries no digests and
//! is safe for transcripts, logs, and dashboards.

use serde::Serialize;

use crate::exact_transition::{
    TraceComparison, TraceOutcome, TransitionError, TransitionTrace, compare_traces,
};
use crate::execution::ExactStateDigest;

/// Digest-free status safe for ordinary consumers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublicDivergenceStatus {
    /// Classification label.
    pub outcome: String,
    /// Last verified equal boundary.
    pub last_equal_ordinal: Option<u64>,
    /// First observed unequal boundary.
    pub first_unequal_ordinal: Option<u64>,
    /// Number of aligned boundaries examined.
    pub compared_records: usize,
    /// Boundaries present in one trace but not the other.
    pub unobserved_records: usize,
}

/// Bounded diagnosis; exact identities are privileged and must not be published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceDiagnosis {
    /// Classification.
    pub outcome: TraceOutcome,
    /// Last verified equal boundary.
    pub last_equal_ordinal: Option<u64>,
    /// First observed unequal boundary.
    pub first_unequal_ordinal: Option<u64>,
    /// Number of aligned boundaries examined.
    pub compared_records: usize,
    /// Boundaries present in one trace but not the other.
    pub unobserved_records: usize,
    /// Privileged exact state identity observed after the expected boundary, when present.
    pub expected_state_digest: Option<ExactStateDigest>,
    /// Privileged exact state identity observed after the actual boundary, when present.
    pub actual_state_digest: Option<ExactStateDigest>,
}

impl TraceDiagnosis {
    /// Returns the digest-free status for public surfaces.
    #[must_use]
    pub fn public_status(&self) -> PublicDivergenceStatus {
        PublicDivergenceStatus {
            outcome: self.outcome.as_str().to_owned(),
            last_equal_ordinal: self.last_equal_ordinal,
            first_unequal_ordinal: self.first_unequal_ordinal,
            compared_records: self.compared_records,
            unobserved_records: self.unobserved_records,
        }
    }

    /// Reports whether privileged identities were recorded for this diagnosis.
    #[must_use]
    pub const fn has_privileged_identities(&self) -> bool {
        self.expected_state_digest.is_some() || self.actual_state_digest.is_some()
    }
}

impl TraceOutcome {
    /// Returns the stable label used in public status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdenticalOverRecordedRange => "identical_over_recorded_range",
            Self::DifferentAction => "different_action",
            Self::DifferentExternalInput => "different_external_input",
            Self::StateDivergence => "state_divergence",
            Self::MissingCapture => "missing_capture",
            Self::RestoreMismatch => "restore_mismatch",
            Self::IncompatibleProfile => "incompatible_profile",
            Self::UnalignedTrace => "unaligned_trace",
            Self::InsufficientCoverage => "insufficient_coverage",
        }
    }
}

/// Compares two traces and returns a bounded diagnosis with privileged identities when available.
pub fn diagnose_traces(
    expected: &TransitionTrace,
    actual: &TransitionTrace,
) -> Result<TraceDiagnosis, TransitionError> {
    let comparison: TraceComparison = compare_traces(expected, actual)?;
    let (expected_state_digest, actual_state_digest) = match comparison.first_unequal_ordinal {
        Some(ordinal) => (state_after(expected, ordinal), state_after(actual, ordinal)),
        None => (None, None),
    };
    Ok(TraceDiagnosis {
        outcome: comparison.outcome,
        last_equal_ordinal: comparison.last_equal_ordinal,
        first_unequal_ordinal: comparison.first_unequal_ordinal,
        compared_records: comparison.compared_records,
        unobserved_records: comparison.unobserved_records,
        expected_state_digest,
        actual_state_digest,
    })
}

fn state_after(trace: &TransitionTrace, ordinal: u64) -> Option<ExactStateDigest> {
    trace
        .records
        .iter()
        .find(|record| record.ordinal == ordinal)
        .map(|record| record.after.clone())
}
