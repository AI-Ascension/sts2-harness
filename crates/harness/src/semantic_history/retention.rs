// SPDX-License-Identifier: MIT

//! Explicit retention over a retained history, and the disclosure a pruned span must keep.
//!
//! Retention here is reference-aware in the same sense the exact checkpoint store's sweep is: a
//! record that another retained record still names as its stated causal parent survives the prune
//! even when the policy would otherwise select it, so a prune can never leave a history holding a
//! cause that is no longer there. The closure is transitive, because pinning an ancestor makes that
//! ancestor a survivor whose own stated parent must then be pinned too.
//!
//! What the policy does select is disclosed rather than deleted. The gameplay value is destroyed and
//! the sequence keeps its number as a declared gap carrying the retention label, so a pruned span
//! reads as "this boundary was not retained" instead of as a zero that was measured, an empty result
//! labelled complete, or an event that never happened. A plan is computed against the exact bytes of
//! one history and is refused when that history has moved since, so an operator cannot apply a
//! preview of a history that is no longer the one in front of them.
//!
//! The policy reuses the shape of the branch store's own retention owner - a protection an operator
//! must disable explicitly, plus a bound below which nothing is pruned. The bound is a count of
//! retained records rather than an age because this store keeps no clock, and that difference is
//! stated here rather than papered over.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::record::{
    SemanticCaptureWindow, SemanticCoverageInterval, SemanticEventCoverage, SemanticEventRecord,
};
use super::roles::SemanticCoverageStatus;
use super::scope::SEMANTIC_MAX_INTERVALS;

/// The coverage label a record carries once retention has replaced the value it held.
pub const SEMANTIC_RETENTION_LABEL: &str = "retention";

/// Operator-selected retention policy for explicit history pruning.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticRetentionPolicy {
    /// Keep observed gameplay detail unless an operator explicitly disables this protection.
    pub retain_observed: bool,
    /// Always retain this many of the most recent records, whatever else the policy permits.
    pub retain_latest: usize,
}

impl SemanticRetentionPolicy {
    /// The fail-closed default: observed detail is retained and nothing is eligible for pruning.
    #[must_use]
    pub const fn protective() -> Self {
        Self {
            retain_observed: true,
            retain_latest: 0,
        }
    }
}

impl Default for SemanticRetentionPolicy {
    fn default() -> Self {
        Self::protective()
    }
}

/// One explicit prune request over a single retained history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticPruneRequest {
    /// Branch whose history the policy applies to.
    pub branch_id: String,
    /// Policy applied to the selection.
    pub policy: SemanticRetentionPolicy,
}

/// What a policy would retain, what it would disclose, and what it must keep either way.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPrunePlan {
    /// Branch this plan was computed for.
    pub branch_id: String,
    /// Digest of the history these sequences were read from; another history refuses the plan.
    pub history_digest: String,
    /// Observed sequences the policy selects for retention-pruning, ascending.
    pub prunable_sequences: Vec<u64>,
    /// Sequences kept because a record that survives still names them as its stated parent.
    pub pinned_sequences: Vec<u64>,
    /// Observed sequences the policy retains on its own terms, neither pruned nor pinned.
    pub retained_sequences: Vec<u64>,
}

impl SemanticPrunePlan {
    /// Returns whether this plan would disclose nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.prunable_sequences.is_empty()
    }
}

/// A stable digest of one history's retained bytes, so a plan is tied to the state it was read from.
pub(super) fn digest_history(
    branch_id: &str,
    records: &[SemanticEventRecord],
) -> SemanticHistoryResult<String> {
    let bytes = serde_json::to_vec(&(branch_id, records))
        .map_err(|_| SemanticHistoryError::new(Refusal::Storage))?;
    Ok(crate::sha256_hex(bytes))
}

/// Computes what a policy would select, without changing anything.
///
/// A record another surviving record still names as its stated parent is never selected, and that
/// protection closes over the ancestor chain, so the resulting plan can always be applied without
/// leaving a stated cause unreachable.
pub(super) fn plan_prune(
    branch_id: &str,
    records: &[SemanticEventRecord],
    policy: &SemanticRetentionPolicy,
) -> SemanticHistoryResult<SemanticPrunePlan> {
    let digest = digest_history(branch_id, records)?;
    let positions = records
        .iter()
        .map(|record| (record.event_id(), record.event.sequence))
        .collect::<BTreeMap<_, _>>();
    let mut prunable = eligible_sequences(records, policy);
    let mut pinned = BTreeSet::new();
    loop {
        let mut pinned_again = false;
        for record in records {
            if prunable.contains(&record.event.sequence) {
                continue;
            }
            let Some(parent) = record
                .event
                .causal_parent
                .as_ref()
                .and_then(|parent| parent.stated_parent())
            else {
                continue;
            };
            let Some(sequence) = positions.get(parent) else {
                continue;
            };
            if prunable.remove(sequence) {
                pinned.insert(*sequence);
                pinned_again = true;
            }
        }
        if !pinned_again {
            break;
        }
    }
    let retained_sequences = records
        .iter()
        .filter(|record| record.is_observed())
        .map(|record| record.event.sequence)
        .filter(|sequence| !prunable.contains(sequence) && !pinned.contains(sequence))
        .collect();
    Ok(SemanticPrunePlan {
        branch_id: branch_id.to_owned(),
        history_digest: digest,
        prunable_sequences: prunable.iter().copied().collect(),
        pinned_sequences: pinned.iter().copied().collect(),
        retained_sequences,
    })
}

/// Applies a plan to one history, disclosing every span it selects.
///
/// The disclosure is the point: each selected record keeps its identity and its sequence number and
/// loses every gameplay value, and the window gains a declared span covering it, so the gap is
/// visible to a reader that never saw the prune.
pub(super) fn apply_prune(
    records: &[SemanticEventRecord],
    window: &SemanticCaptureWindow,
    plan: &SemanticPrunePlan,
) -> SemanticHistoryResult<(Vec<SemanticEventRecord>, SemanticCaptureWindow)> {
    if plan.is_empty() {
        return Err(SemanticHistoryError::new(Refusal::NothingPrunable));
    }
    let pruned = plan
        .prunable_sequences
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut disclosed_records = Vec::with_capacity(records.len());
    for record in records {
        if !pruned.contains(&record.event.sequence) {
            disclosed_records.push(record.clone());
            continue;
        }
        let mut event = record.event.clone();
        event.coverage = SemanticEventCoverage {
            status: SemanticCoverageStatus::Dropped,
            label: Some(SEMANTIC_RETENTION_LABEL.to_owned()),
        };
        event.kind = None;
        event.origin = None;
        event.subjects.clear();
        event.causal_parent = None;
        event.value = None;
        event.reference = None;
        event.label = None;
        disclosed_records.push(SemanticEventRecord {
            binding: record.binding.clone(),
            scope: record.scope.clone(),
            event,
        });
    }
    let intervals = intervals_covering(&pruned);
    let disclosed = SemanticCaptureWindow {
        capture_start_sequence: window.capture_start_sequence,
        history_before_capture: window.history_before_capture,
        intervals: merge_intervals(&window.intervals, &intervals)?,
    };
    super::window::validate_window(&disclosed)?;
    Ok((disclosed_records, disclosed))
}

/// The declared spans a set of selected sequences needs, one per consecutive run.
pub(super) fn intervals_covering(pruned: &BTreeSet<u64>) -> Vec<SemanticCoverageInterval> {
    let mut intervals = Vec::new();
    let mut run: Option<(u64, u64)> = None;
    for sequence in pruned {
        run = match run {
            Some((first, last)) if last.saturating_add(1) == *sequence => Some((first, *sequence)),
            Some((first, last)) => {
                intervals.push((first, last));
                Some((*sequence, *sequence))
            }
            None => Some((*sequence, *sequence)),
        };
    }
    if let Some((first, last)) = run {
        intervals.push((first, last));
    }
    intervals
        .into_iter()
        .map(|(first_sequence, last_sequence)| SemanticCoverageInterval {
            status: SemanticCoverageStatus::Dropped,
            first_sequence,
            last_sequence,
        })
        .collect()
}

/// Combines two spans lists into one ascending, non-overlapping, in-bound list.
///
/// The two inputs are disjoint by construction - a producer never declares a span over a record
/// retention already replaced - so this is a merge rather than a rewrite, and the bound is checked
/// before anything is written.
pub(super) fn merge_intervals(
    first: &[SemanticCoverageInterval],
    second: &[SemanticCoverageInterval],
) -> SemanticHistoryResult<Vec<SemanticCoverageInterval>> {
    let mut intervals = Vec::with_capacity(first.len() + second.len());
    intervals.extend_from_slice(first);
    intervals.extend_from_slice(second);
    intervals.sort_by_key(|interval| interval.first_sequence);
    if intervals.len() > SEMANTIC_MAX_INTERVALS {
        return Err(SemanticHistoryError::new(Refusal::TooManyIntervals));
    }
    let mut previous_end: Option<u64> = None;
    for interval in &intervals {
        if previous_end.is_some_and(|end| interval.first_sequence <= end) {
            return Err(SemanticHistoryError::new(Refusal::OverlappingIntervals));
        }
        previous_end = Some(previous_end.map_or(interval.last_sequence, |end| {
            end.max(interval.last_sequence)
        }));
    }
    Ok(intervals)
}

fn eligible_sequences(
    records: &[SemanticEventRecord],
    policy: &SemanticRetentionPolicy,
) -> BTreeSet<u64> {
    if policy.retain_observed {
        return BTreeSet::new();
    }
    let highest = records
        .iter()
        .map(|record| record.event.sequence)
        .max()
        .unwrap_or(0);
    let cutoff = highest.saturating_sub(u64::try_from(policy.retain_latest).unwrap_or(u64::MAX));
    records
        .iter()
        .filter(|record| record.is_observed())
        .map(|record| record.event.sequence)
        .filter(|sequence| *sequence <= cutoff)
        .collect()
}
