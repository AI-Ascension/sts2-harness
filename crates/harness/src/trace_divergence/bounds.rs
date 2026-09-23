// SPDX-License-Identifier: MIT

//! Explicit record/entry/byte bounds and their truncation accounting.
//!
//! A diagnostic owner must be able to bound the work and the size of any privileged diff before
//! touching an adversarial trace. Every bound here is caller-chosen and reported back, so a
//! truncated result is never mistaken for a complete one.

use serde::Serialize;

use crate::exact_transition::{TransitionError, TransitionRecord, TransitionTrace};

/// Default maximum boundaries examined per comparison.
pub const DEFAULT_MAX_RECORDS: usize = 10_000;
/// Default maximum privileged field differences retained.
pub const DEFAULT_MAX_ENTRIES: usize = 64;
/// Default maximum retained bytes across privileged field differences.
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024;

/// Explicit, caller-chosen bounds applied before and during diagnosis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DivergenceLimits {
    /// Maximum boundaries examined from each trace.
    pub max_records: usize,
    /// Maximum privileged field-difference entries retained.
    pub max_entries: usize,
    /// Maximum bytes retained across privileged field-difference entries.
    pub max_bytes: usize,
}

impl Default for DivergenceLimits {
    fn default() -> Self {
        Self {
            max_records: DEFAULT_MAX_RECORDS,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl DivergenceLimits {
    /// Builds explicit bounds; a zero bound is refused by admission.
    #[must_use]
    pub const fn new(max_records: usize, max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_records,
            max_entries,
            max_bytes,
        }
    }

    /// Reports whether every bound admits at least one unit of work.
    #[must_use]
    pub const fn is_nonzero(self) -> bool {
        self.max_records > 0 && self.max_entries > 0 && self.max_bytes > 0
    }
}

/// Explicit truncation accounting for the applied bounds.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct LimitTruncation {
    /// Boundaries examined from each bounded view.
    pub records_examined: usize,
    /// Whether either trace had boundaries beyond the record bound.
    pub records_truncated: bool,
    /// Privileged field-difference entries retained.
    pub entries_kept: usize,
    /// Privileged field-difference entries dropped by the entry or byte bound.
    pub entries_dropped: usize,
    /// Bytes retained across the privileged field-difference entries.
    pub bytes_kept: usize,
    /// Whether any entry was dropped because a bound was reached.
    pub bytes_excluded: bool,
}

/// One privileged field difference at the failing boundary.
///
/// Values may be exact digests, so this type is only ever returned from a privileged caller path
/// and must not be published to a transcript, log or model input.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FieldDifference {
    /// Canonical comparison field name.
    pub field: &'static str,
    /// Expected value rendered as text.
    pub expected: String,
    /// Actual value rendered as text.
    pub actual: String,
}

/// Returns a head of `trace` bounded to `max_records`, revalidating the shortened chain.
pub(crate) fn bounded_view(
    trace: &TransitionTrace,
    max_records: usize,
) -> Result<TransitionTrace, TransitionError> {
    trace.validate()?;
    if trace.records.len() <= max_records {
        return Ok(trace.clone());
    }
    let mut bounded = trace.clone();
    bounded.records.truncate(max_records);
    bounded.validate()?;
    Ok(bounded)
}

/// Rejects nothing but returns the rendered pairs of every field that can differ.
fn field_pairs(record: &TransitionRecord) -> Vec<(&'static str, String)> {
    vec![
        ("boundary_kind", record.boundary_kind.clone()),
        ("boundary_phase", record.boundary_phase.clone()),
        ("before", record.before.as_str().to_owned()),
        ("after", record.after.as_str().to_owned()),
        ("action_key", record.action_key.clone()),
        ("action_schema", record.action_schema.clone()),
        (
            "catalog_witness",
            record
                .catalog_witness
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
        ),
        (
            "external_input_digest",
            record
                .external_input_digest
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
        ),
    ]
}

/// Builds a bounded privileged diff between two aligned records.
///
/// Returns `(kept, dropped, bytes_kept, bytes_excluded)`.
pub(crate) fn bounded_field_diff(
    expected: &TransitionRecord,
    actual: &TransitionRecord,
    limits: &DivergenceLimits,
) -> (Vec<FieldDifference>, usize, usize, bool) {
    let mut kept: Vec<FieldDifference> = Vec::new();
    let mut dropped = 0usize;
    let mut bytes = 0usize;
    let mut excluded = false;
    let left = field_pairs(expected);
    let right = field_pairs(actual);
    for ((field, expected_value), (_, actual_value)) in left.into_iter().zip(right) {
        if expected_value == actual_value {
            continue;
        }
        let entry_bytes = field.len() + expected_value.len() + actual_value.len();
        if kept.len() >= limits.max_entries || bytes.saturating_add(entry_bytes) > limits.max_bytes
        {
            dropped += 1;
            excluded = true;
            continue;
        }
        bytes += entry_bytes;
        kept.push(FieldDifference {
            field,
            expected: expected_value,
            actual: actual_value,
        });
    }
    (kept, dropped, bytes, excluded)
}
