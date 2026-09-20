// SPDX-License-Identifier: MIT

//! Bounded filtering and pagination over a retained history.

use serde::{Deserialize, Serialize};

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::record::SemanticEventRecord;
use super::roles::SemanticCoverageStatus;
use super::scope::{SEMANTIC_MAX_PAGE_ITEMS, SemanticHistoryFence};
use super::vocabulary::{SemanticEventKind, SemanticEventOrigin};

/// One bounded filter over a retained history.
///
/// Every field is a narrowing, never a substitution: an unset field means "any", and a set field
/// that matches nothing yields an empty page rather than a widened one.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SemanticEventListQuery {
    /// Restrict to one event kind.
    pub kind: Option<SemanticEventKind>,
    /// Restrict to one origin.
    pub origin: Option<SemanticEventOrigin>,
    /// Restrict to one coverage status, so gaps can be listed deliberately.
    pub coverage: Option<SemanticCoverageStatus>,
    /// Restrict to events naming this subject identity.
    pub subject_id: Option<String>,
    /// Restrict to events at or after this sequence number.
    pub from_sequence: Option<u64>,
    /// Restrict to events at or before this sequence number.
    pub to_sequence: Option<u64>,
    /// Maximum entries to return; bounded by [`SEMANTIC_MAX_PAGE_ITEMS`].
    pub limit: usize,
    /// Continuation token from a previous page.
    pub continuation: Option<SemanticHistoryContinuation>,
}

/// The position a page stopped at, so the next page resumes exactly there.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryContinuation {
    /// Sequence number of the last entry returned.
    pub after_sequence: u64,
}

/// One bounded page of a retained history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticEventPage {
    /// The records in ascending sequence order.
    pub records: Vec<SemanticEventRecord>,
    /// The continuation to resume from, present only when more entries remain.
    pub continuation: Option<SemanticHistoryContinuation>,
    /// Total entries the filter matched, so a caller can see how much it did not read.
    pub matched: usize,
}

impl SemanticEventPage {
    /// Returns whether this page is the last one for its filter.
    #[must_use]
    pub fn is_final(&self) -> bool {
        self.continuation.is_none()
    }
}

/// Applies a bounded filter and page to records already admitted for one scope.
pub fn page_history(
    records: &[SemanticEventRecord],
    fence: &SemanticHistoryFence,
    query: &SemanticEventListQuery,
) -> SemanticHistoryResult<SemanticEventPage> {
    if query.limit == 0 || query.limit > SEMANTIC_MAX_PAGE_ITEMS {
        return Err(SemanticHistoryError::new(Refusal::PageTooLarge));
    }
    if let (Some(from), Some(to)) = (query.from_sequence, query.to_sequence)
        && from > to
    {
        return Err(SemanticHistoryError::new(Refusal::NonMonotonicSequence));
    }
    let after = query
        .continuation
        .as_ref()
        .map(|continuation| continuation.after_sequence);
    let mut matched = Vec::new();
    for record in records {
        if !fence.admits(&record.scope) {
            return Err(SemanticHistoryError::about(
                Refusal::StaleFence,
                record.event_id(),
            ));
        }
        if after.is_some_and(|after| record.event.sequence <= after) {
            continue;
        }
        if matches_query(record, query) {
            matched.push(record.clone());
        }
    }
    let matched_total = matched.len();
    let more = matched_total > query.limit;
    matched.truncate(query.limit);
    let continuation = more.then(|| SemanticHistoryContinuation {
        after_sequence: matched.last().map_or(0, |record| record.event.sequence),
    });
    Ok(SemanticEventPage {
        records: matched,
        continuation,
        matched: matched_total,
    })
}

fn matches_query(record: &SemanticEventRecord, query: &SemanticEventListQuery) -> bool {
    let event = &record.event;
    if query.kind.is_some_and(|kind| event.kind != Some(kind)) {
        return false;
    }
    if query
        .origin
        .is_some_and(|origin| event.origin != Some(origin))
    {
        return false;
    }
    if query
        .coverage
        .is_some_and(|coverage| event.coverage.status != coverage)
    {
        return false;
    }
    if query
        .from_sequence
        .is_some_and(|from| event.sequence < from)
    {
        return false;
    }
    if query.to_sequence.is_some_and(|to| event.sequence > to) {
        return false;
    }
    if let Some(subject_id) = &query.subject_id
        && !event
            .subjects
            .iter()
            .any(|subject| &subject.subject_id == subject_id)
    {
        return false;
    }
    true
}
