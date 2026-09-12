// SPDX-License-Identifier: MIT

//! Semantic transition records and earliest-divergence comparison.
//!
//! A transition record commits to one committed boundary's exact state before and after, its
//! logical action and schema, the controlled external input, and the previous commitment. Provider
//! names, wall-clock times, transport operation identifiers, and branch occurrence identifiers stay
//! outside the semantic comparison. Comparison is a linear aligned scan: matching endpoints never
//! prove that earlier states matched, because trajectories can diverge and reconverge.

use sha2::{Digest, Sha256};

use crate::execution::{BlobDigest, ExactStateDigest};

mod error;

pub use error::TransitionError;

/// Version bound into every transition commitment.
pub const TRANSITION_COMMITMENT_VERSION: &str = "asc-transition:v1";
/// Serialized prefix of a transition commitment.
pub const TRANSITION_COMMITMENT_PREFIX: &str = "asc-transition:v1:sha256:";
/// Domain separator for the transition commitment.
pub const TRANSITION_DOMAIN: &[u8] = b"AI-ASCENSION/TRANSITION/v1\0";
/// Maximum records accepted in one trace.
pub const MAX_TRANSITION_RECORDS: usize = 100_000;
/// Maximum length of a boundary, action, or profile label.
pub const MAX_TRANSITION_LABEL_BYTES: usize = 256;

/// One committed boundary in a deterministic run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionRecord {
    /// Ordinal of this boundary within the trace.
    pub ordinal: u64,
    /// Boundary kind, for example `decision` or `effect`.
    pub boundary_kind: String,
    /// Game phase at the boundary.
    pub boundary_phase: String,
    /// Exact state identity immediately before the action.
    pub before: ExactStateDigest,
    /// Exact state identity after the action committed.
    pub after: ExactStateDigest,
    /// Stable semantic action identity.
    pub action_key: String,
    /// Versioned semantic action schema identifier.
    pub action_schema: String,
    /// Semantic legal-catalog witness, when captured.
    pub catalog_witness: Option<String>,
    /// Controlled external input digest, when the boundary consumed one.
    pub external_input_digest: Option<String>,
    /// Commitment of the previous record; `None` only at genesis.
    pub previous_commitment: Option<String>,
}

impl TransitionRecord {
    /// Returns the domain-separated commitment over this record and its predecessor.
    pub fn commitment(&self) -> Result<String, TransitionError> {
        self.validate()?;
        let mut payload = Vec::new();
        let fields = [
            TRANSITION_COMMITMENT_VERSION.to_owned(),
            self.ordinal.to_string(),
            self.boundary_kind.clone(),
            self.boundary_phase.clone(),
            self.before.as_str().to_owned(),
            self.after.as_str().to_owned(),
            self.action_key.clone(),
            self.action_schema.clone(),
            self.catalog_witness
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
            self.external_input_digest
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
            self.previous_commitment
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
        ];
        for field in fields {
            payload.extend_from_slice(field.as_bytes());
            payload.push(0);
        }
        let mut hasher = Sha256::new();
        hasher.update(TRANSITION_DOMAIN);
        hasher.update(&payload);
        let hex: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(format!("{TRANSITION_COMMITMENT_PREFIX}{hex}"))
    }

    /// Validates labels and digest namespaces.
    pub fn validate(&self) -> Result<(), TransitionError> {
        for label in [
            &self.boundary_kind,
            &self.boundary_phase,
            &self.action_key,
            &self.action_schema,
        ] {
            if !valid_label(label) {
                return Err(TransitionError::InvalidRecord);
            }
        }
        for digest in [
            self.catalog_witness.as_deref(),
            self.external_input_digest.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            BlobDigest::parse(digest).map_err(|_| TransitionError::InvalidDigest)?;
        }
        if let Some(previous) = &self.previous_commitment
            && valid_commitment(previous).is_none()
        {
            return Err(TransitionError::InvalidDigest);
        }
        Ok(())
    }
}

/// A deterministically recorded sequence of committed boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionTrace {
    /// Compatibility/profile identifier the trace was recorded under.
    pub profile: String,
    /// Exact state identity at the start of the trace.
    pub source_state: ExactStateDigest,
    /// Committed boundaries in recorded order.
    pub records: Vec<TransitionRecord>,
}

impl TransitionTrace {
    /// Validates genesis binding, ordinal monotonicity, and the commitment chain.
    pub fn validate(&self) -> Result<(), TransitionError> {
        if !valid_label(&self.profile) || self.records.len() > MAX_TRANSITION_RECORDS {
            return Err(TransitionError::InvalidTrace);
        }
        for (index, record) in self.records.iter().enumerate() {
            record.validate()?;
            if index == 0 {
                if record.before != self.source_state || record.previous_commitment.is_some() {
                    return Err(TransitionError::BrokenCommitmentChain);
                }
                continue;
            }
            let previous = &self.records[index - 1];
            if record.ordinal <= previous.ordinal {
                return Err(TransitionError::NonMonotonicOrdinal);
            }
            if record.previous_commitment.as_deref() != Some(previous.commitment()?.as_str()) {
                return Err(TransitionError::BrokenCommitmentChain);
            }
        }
        Ok(())
    }
}

/// Comparison outcome for two aligned traces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceOutcome {
    /// Every recorded and observed boundary matched.
    IdenticalOverRecordedRange,
    /// The traces chose different logical actions.
    DifferentAction,
    /// The controlled external input differed.
    DifferentExternalInput,
    /// Exact state differed with matching actions and inputs.
    StateDivergence,
    /// A required capture or catalog witness is absent.
    MissingCapture,
    /// Genesis state differed, so the traces do not share a verified origin.
    RestoreMismatch,
    /// Profiles are not comparable.
    IncompatibleProfile,
    /// Boundary coordinates do not align.
    UnalignedTrace,
    /// The expected trace contains no recorded boundary.
    InsufficientCoverage,
}

/// Bounded comparison report with the last equal and first unequal boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceComparison {
    /// Classification of the comparison.
    pub outcome: TraceOutcome,
    /// Ordinal of the last verified equal boundary.
    pub last_equal_ordinal: Option<u64>,
    /// Ordinal of the first observed unequal boundary.
    pub first_unequal_ordinal: Option<u64>,
    /// Number of aligned boundaries examined.
    pub compared_records: usize,
    /// Boundaries present in one trace but not the other.
    pub unobserved_records: usize,
}

/// Compares two traces by aligned linear scan without assuming endpoint equality.
pub fn compare_traces(
    expected: &TransitionTrace,
    actual: &TransitionTrace,
) -> Result<TraceComparison, TransitionError> {
    expected.validate()?;
    actual.validate()?;
    if expected.profile != actual.profile {
        return Ok(report(TraceOutcome::IncompatibleProfile, None, None, 0, 0));
    }
    if expected.records.is_empty() {
        return Ok(report(TraceOutcome::InsufficientCoverage, None, None, 0, 0));
    }
    if expected.source_state != actual.source_state {
        return Ok(report(
            TraceOutcome::RestoreMismatch,
            None,
            expected.records.first().map(|record| record.ordinal),
            0,
            0,
        ));
    }
    let shared = expected.records.len().min(actual.records.len());
    for index in 0..shared {
        let left = &expected.records[index];
        let right = &actual.records[index];
        let last_equal = index
            .checked_sub(1)
            .map(|prior| expected.records[prior].ordinal);
        if left.ordinal != right.ordinal
            || left.boundary_kind != right.boundary_kind
            || left.boundary_phase != right.boundary_phase
        {
            return Ok(report(
                TraceOutcome::UnalignedTrace,
                last_equal,
                Some(left.ordinal),
                index,
                0,
            ));
        }
        let outcome = classify(left, right);
        if let Some(outcome) = outcome {
            return Ok(report(outcome, last_equal, Some(left.ordinal), index, 0));
        }
    }
    if actual.records.len() < expected.records.len() {
        let last_equal = Some(expected.records[shared - 1].ordinal);
        let missing = expected.records[shared].ordinal;
        return Ok(report(
            TraceOutcome::MissingCapture,
            last_equal,
            Some(missing),
            shared,
            expected.records.len() - shared,
        ));
    }
    let last_equal = expected.records.last().map(|record| record.ordinal);
    let unobserved = actual.records.len() - expected.records.len();
    Ok(report(
        TraceOutcome::IdenticalOverRecordedRange,
        last_equal,
        None,
        shared,
        unobserved,
    ))
}

fn classify(left: &TransitionRecord, right: &TransitionRecord) -> Option<TraceOutcome> {
    if left.action_key != right.action_key || left.action_schema != right.action_schema {
        return Some(TraceOutcome::DifferentAction);
    }
    if left.external_input_digest != right.external_input_digest {
        return Some(TraceOutcome::DifferentExternalInput);
    }
    match (&left.catalog_witness, &right.catalog_witness) {
        (Some(left_witness), Some(right_witness)) if left_witness != right_witness => {
            return Some(TraceOutcome::StateDivergence);
        }
        (Some(_), None) | (None, Some(_)) => return Some(TraceOutcome::MissingCapture),
        _ => {}
    }
    if left.before != right.before || left.after != right.after {
        return Some(TraceOutcome::StateDivergence);
    }
    None
}

fn report(
    outcome: TraceOutcome,
    last_equal_ordinal: Option<u64>,
    first_unequal_ordinal: Option<u64>,
    compared_records: usize,
    unobserved_records: usize,
) -> TraceComparison {
    TraceComparison {
        outcome,
        last_equal_ordinal,
        first_unequal_ordinal,
        compared_records,
        unobserved_records,
    }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TRANSITION_LABEL_BYTES && !value.contains('\0')
}

fn valid_commitment(value: &str) -> Option<()> {
    let hex = value.strip_prefix(TRANSITION_COMMITMENT_PREFIX)?;
    let lowercase = hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (hex.len() == 64 && lowercase).then_some(())
}
