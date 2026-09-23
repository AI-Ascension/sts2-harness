// SPDX-License-Identifier: MIT

//! Shortest recorded replay prefix ending at a failing boundary.
//!
//! The reproducer is a prefix extraction, not a delta debugger: it replays the recorded
//! boundaries from genesis through the reported boundary and keeps the original evidence by
//! refusing to mutate it. Limits are fail-closed here, because a clipped prefix that does not reach
//! its boundary would be a false reproducer rather than a shorter one.

use crate::exact_transition::{TransitionRecord, TransitionTrace};
use crate::execution::ExactStateDigest;

use super::bounds::DivergenceLimits;

/// Refusal reasons for a bounded reproducer prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReproducerError {
    /// The requested boundary ordinal is not present in the source trace.
    BoundaryNotRecorded,
    /// The prefix or its boundary would exceed an explicit bound.
    PrefixTooLarge,
    /// The prefix is not a byte-identical head of the source trace.
    PrefixMismatch,
    /// The source trace no longer matches its recorded boundary commitment.
    SourceChanged,
}

/// A bounded reproducer: the recorded prefix ending at the failing boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReproducerPrefix {
    /// Compatibility/profile identifier of the source trace.
    pub profile: String,
    /// Exact state identity at the start of the prefix.
    pub source_state: ExactStateDigest,
    /// Ordinal of the last included boundary.
    pub boundary_ordinal: u64,
    /// Commitment of the last included boundary.
    pub boundary_commitment: String,
    /// Recorded boundaries from genesis through the failing boundary.
    pub records: Vec<TransitionRecord>,
    /// Deterministic encoded size of the prefix in bytes.
    pub byte_len: usize,
}

impl ReproducerPrefix {
    /// Verifies the prefix is a byte-identical head of `source` ending at the reported boundary.
    pub fn validate_against_source(&self, source: &TransitionTrace) -> Result<(), ReproducerError> {
        if source.profile != self.profile || source.source_state != self.source_state {
            return Err(ReproducerError::SourceChanged);
        }
        if self.records.is_empty() {
            return Err(ReproducerError::BoundaryNotRecorded);
        }
        let head = &source.records[..self.records.len().min(source.records.len())];
        if head.len() != self.records.len() || head != self.records.as_slice() {
            return Err(ReproducerError::PrefixMismatch);
        }
        self.validate_boundary()
    }

    fn validate_boundary(&self) -> Result<(), ReproducerError> {
        let last = self
            .records
            .last()
            .ok_or(ReproducerError::BoundaryNotRecorded)?;
        if last.ordinal != self.boundary_ordinal {
            return Err(ReproducerError::BoundaryNotRecorded);
        }
        let commitment = last
            .commitment()
            .map_err(|_| ReproducerError::SourceChanged)?;
        if commitment != self.boundary_commitment {
            return Err(ReproducerError::SourceChanged);
        }
        Ok(())
    }
}

/// Exports the recorded prefix ending at `boundary_ordinal`, bounded by `limits`.
pub fn export_reproducer_prefix(
    trace: &TransitionTrace,
    boundary_ordinal: u64,
    limits: &DivergenceLimits,
) -> Result<ReproducerPrefix, ReproducerError> {
    trace
        .validate()
        .map_err(|_| ReproducerError::SourceChanged)?;
    let index = trace
        .records
        .iter()
        .position(|record| record.ordinal == boundary_ordinal)
        .ok_or(ReproducerError::BoundaryNotRecorded)?;
    if index >= limits.max_records {
        return Err(ReproducerError::PrefixTooLarge);
    }
    let records = trace.records[..=index].to_vec();
    let byte_len = encoded_len(&records, &trace.profile);
    if byte_len > limits.max_bytes {
        return Err(ReproducerError::PrefixTooLarge);
    }
    let boundary_commitment = records
        .last()
        .ok_or(ReproducerError::BoundaryNotRecorded)?
        .commitment()
        .map_err(|_| ReproducerError::SourceChanged)?;
    Ok(ReproducerPrefix {
        profile: trace.profile.clone(),
        source_state: trace.source_state.clone(),
        boundary_ordinal,
        boundary_commitment,
        records,
        byte_len,
    })
}

fn encoded_len(records: &[TransitionRecord], profile: &str) -> usize {
    let mut total = profile.len();
    for record in records {
        total += record.boundary_kind.len()
            + record.boundary_phase.len()
            + record.action_key.len()
            + record.action_schema.len()
            + record.before.as_str().len()
            + record.after.as_str().len()
            + record.catalog_witness.as_ref().map_or(0, String::len)
            + record.external_input_digest.as_ref().map_or(0, String::len)
            + record.previous_commitment.as_ref().map_or(0, String::len)
            + 16;
    }
    total
}
