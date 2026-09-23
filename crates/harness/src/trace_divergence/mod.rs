// SPDX-License-Identifier: MIT

//! Offline admission and bounded first-divergence diagnosis for replay bundles.
//!
//! `crate::exact_transition` already compares two validated traces by aligned linear scan, and
//! `crate::trace_diagnosis` projects the result into a privileged and a public face. This module
//! adds the two pieces a diagnostic owner still needs around that comparison:
//!
//! * an immutable [`TraceBundleManifest`] per bundle, with a closure commitment over the recorded
//!   boundary chain, admitted by [`admit_traces`] *before* any alignment; and
//! * a bounded [`AdmittedDiagnosis`] that reports explicit record/entry/byte truncation and, on
//!   divergence, a [`ReproducerPrefix`] that replays only up to the failing boundary.
//!
//! Everything here is offline and read-only. Nothing launches a game, mutates a profile, spends
//! provider credits or publishes privileged exact identities: [`AdmittedDiagnosis::public_status`]
//! is digest-free. Native mismatch validation remains an explicit external gate (harness #123).

mod admission;
mod bounds;
mod reproducer;

pub use admission::{AdmissionRefusal, TraceBundleManifest, admit_traces};
pub use bounds::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_RECORDS, DivergenceLimits, FieldDifference,
    LimitTruncation,
};
pub use reproducer::{ReproducerError, ReproducerPrefix, export_reproducer_prefix};

use crate::exact_transition::{TransitionError, TransitionTrace};
use crate::trace_diagnosis::{PublicDivergenceStatus, TraceDiagnosis, diagnose_traces};

/// Failure before or during an offline bundle diagnosis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceDivergenceError {
    /// The two bundles cannot be compared at all.
    Refused(AdmissionRefusal),
    /// A bounded reproducer could not be produced for a reported divergence.
    Reproducer(ReproducerError),
}

/// A bounded, admitted diagnosis with an optional reproducer prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedDiagnosis {
    /// Read-only diagnosis from the admitted comparison.
    pub diagnosis: TraceDiagnosis,
    /// Explicit truncation accounting for the applied bounds.
    pub truncation: LimitTruncation,
    /// Bounded privileged field differences at the failing boundary.
    pub privileged_differences: Vec<FieldDifference>,
    /// Shortest recorded prefix ending at the failing boundary, when one exists.
    pub reproducer: Option<ReproducerPrefix>,
}

impl AdmittedDiagnosis {
    /// Returns the digest-free status safe for transcripts, logs and dashboards.
    #[must_use]
    pub fn public_status(&self) -> PublicDivergenceStatus {
        self.diagnosis.public_status()
    }
}

/// Admits two manifest-bound bundles, compares bounded views and exports a reproducer on divergence.
pub fn diagnose_bundles(
    expected_manifest: &TraceBundleManifest,
    expected: &TransitionTrace,
    actual_manifest: &TraceBundleManifest,
    actual: &TransitionTrace,
    limits: &DivergenceLimits,
) -> Result<AdmittedDiagnosis, TraceDivergenceError> {
    admit_traces(expected_manifest, expected, actual_manifest, actual, limits)
        .map_err(TraceDivergenceError::Refused)?;
    let expected_view = bounded(expected, limits)?;
    let actual_view = bounded(actual, limits)?;
    let records_truncated = expected_view.records.len() < expected.records.len()
        || actual_view.records.len() < actual.records.len();
    let diagnosis =
        diagnose_traces(&expected_view, &actual_view).map_err(refused_as_closure_mismatch)?;
    let (privileged_differences, entries_dropped, bytes_kept, bytes_excluded) =
        bounded_differences(&expected_view, &actual_view, &diagnosis, limits);
    let reproducer = match diagnosis.first_unequal_ordinal {
        Some(ordinal) => Some(
            export_reproducer_prefix(&expected_view, ordinal, limits)
                .map_err(TraceDivergenceError::Reproducer)?,
        ),
        None => None,
    };
    let truncation = LimitTruncation {
        records_examined: diagnosis.compared_records,
        records_truncated,
        entries_kept: privileged_differences.len(),
        entries_dropped,
        bytes_kept,
        bytes_excluded,
    };
    Ok(AdmittedDiagnosis {
        diagnosis,
        truncation,
        privileged_differences,
        reproducer,
    })
}

fn bounded(
    trace: &TransitionTrace,
    limits: &DivergenceLimits,
) -> Result<TransitionTrace, TraceDivergenceError> {
    bounds::bounded_view(trace, limits.max_records).map_err(refused_as_closure_mismatch)
}

fn refused_as_closure_mismatch(_: TransitionError) -> TraceDivergenceError {
    TraceDivergenceError::Refused(AdmissionRefusal::ClosureMismatch)
}

fn bounded_differences(
    expected: &TransitionTrace,
    actual: &TransitionTrace,
    diagnosis: &TraceDiagnosis,
    limits: &DivergenceLimits,
) -> (Vec<FieldDifference>, usize, usize, bool) {
    let Some(ordinal) = diagnosis.first_unequal_ordinal else {
        return (Vec::new(), 0, 0, false);
    };
    let (Some(left), Some(right)) = (record_at(expected, ordinal), record_at(actual, ordinal))
    else {
        return (Vec::new(), 0, 0, false);
    };
    bounds::bounded_field_diff(left, right, limits)
}

fn record_at(
    trace: &TransitionTrace,
    ordinal: u64,
) -> Option<&crate::exact_transition::TransitionRecord> {
    trace
        .records
        .iter()
        .find(|record| record.ordinal == ordinal)
}
