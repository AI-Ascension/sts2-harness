// SPDX-License-Identifier: MIT

//! Immutable trace-bundle manifests and pre-comparison admission.
//!
//! Comparison must not start until both bundles are proven to be the immutable recordings their
//! manifests claim, and proven to describe comparable coverage. Admission binds a manifest to its
//! trace's boundary chain (closure), then refuses when profiles, action-schema revisions or bounded
//! coverage make alignment meaningless.

use serde::Serialize;

use crate::exact_transition::{TransitionError, TransitionRecord, TransitionTrace};
use crate::execution::ExactStateDigest;

use super::bounds::DivergenceLimits;

const MAX_BUNDLE_REF_BYTES: usize = 256;

/// Refusal reasons returned before any alignment is attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionRefusal {
    /// A limit bound was zero, so no comparison could be admitted.
    InvalidLimits,
    /// A bundle reference is empty, oversized or contains a NUL separator.
    InvalidManifest,
    /// A manifest does not bind the supplied trace's boundary chain.
    ClosureMismatch,
    /// The two bundles were recorded under different profiles.
    IncompatibleProfile,
    /// The two bundles declare different action-schema revisions.
    IncompatibleSchema,
    /// The expected bundle records no boundary to compare.
    EmptyCoverage,
}

/// Immutable description of one recorded replay bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceBundleManifest {
    /// Opaque stable reference for the bundle; never a published artifact.
    pub bundle_ref: String,
    /// Compatibility/profile identifier the trace was recorded under.
    pub profile: String,
    /// Sorted, unique action-schema revisions present in the bundle.
    pub action_schema_revisions: Vec<String>,
    /// Exact state identity at the start of the trace.
    pub source_state: ExactStateDigest,
    /// Number of recorded boundaries.
    pub record_count: u64,
    /// Commitment of the last recorded boundary; `None` for an empty trace.
    pub closure_commitment: Option<String>,
}

impl TraceBundleManifest {
    /// Derives a manifest from a validated trace.
    pub fn for_trace(trace: &TransitionTrace, bundle_ref: &str) -> Result<Self, TransitionError> {
        trace.validate()?;
        let closure_commitment = trace
            .records
            .last()
            .map(TransitionRecord::commitment)
            .transpose()?;
        Ok(Self {
            bundle_ref: bundle_ref.to_owned(),
            profile: trace.profile.clone(),
            action_schema_revisions: schema_revisions(trace),
            source_state: trace.source_state.clone(),
            record_count: u64::try_from(trace.records.len())
                .map_err(|_| TransitionError::InvalidTrace)?,
            closure_commitment,
        })
    }

    /// Reports whether this manifest still binds the supplied trace unchanged.
    #[must_use]
    pub fn binds(&self, trace: &TransitionTrace) -> bool {
        let count = u64::try_from(trace.records.len()).unwrap_or(u64::MAX);
        let closure = trace
            .records
            .last()
            .and_then(|record| record.commitment().ok());
        self.profile == trace.profile
            && self.source_state == trace.source_state
            && self.record_count == count
            && self.closure_commitment == closure
            && self.action_schema_revisions == schema_revisions(trace)
    }
}

/// Validates and admits two manifest-bound bundles before any alignment.
pub fn admit_traces(
    expected_manifest: &TraceBundleManifest,
    expected: &TransitionTrace,
    actual_manifest: &TraceBundleManifest,
    actual: &TransitionTrace,
    limits: &DivergenceLimits,
) -> Result<(), AdmissionRefusal> {
    if !limits.is_nonzero() {
        return Err(AdmissionRefusal::InvalidLimits);
    }
    if !valid_bundle_ref(&expected_manifest.bundle_ref)
        || !valid_bundle_ref(&actual_manifest.bundle_ref)
    {
        return Err(AdmissionRefusal::InvalidManifest);
    }
    expected
        .validate()
        .map_err(|_| AdmissionRefusal::ClosureMismatch)?;
    actual
        .validate()
        .map_err(|_| AdmissionRefusal::ClosureMismatch)?;
    if !expected_manifest.binds(expected) || !actual_manifest.binds(actual) {
        return Err(AdmissionRefusal::ClosureMismatch);
    }
    if expected_manifest.profile != actual_manifest.profile {
        return Err(AdmissionRefusal::IncompatibleProfile);
    }
    if expected.records.is_empty() {
        return Err(AdmissionRefusal::EmptyCoverage);
    }
    if expected_manifest.action_schema_revisions != actual_manifest.action_schema_revisions {
        return Err(AdmissionRefusal::IncompatibleSchema);
    }
    Ok(())
}

fn schema_revisions(trace: &TransitionTrace) -> Vec<String> {
    let mut revisions: Vec<String> = trace
        .records
        .iter()
        .map(|record| record.action_schema.clone())
        .collect();
    revisions.sort();
    revisions.dedup();
    revisions
}

fn valid_bundle_ref(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_BUNDLE_REF_BYTES && !value.contains('\0')
}
