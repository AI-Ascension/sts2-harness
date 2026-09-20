// SPDX-License-Identifier: MIT

//! Bounded filter, pagination and summary over one branch's history.

use serde::{Deserialize, Serialize};

use super::{
    Error, MAX_HISTORY_PAGE, SemanticHistoryCoverageStatus, SemanticHistoryCursor,
    SemanticHistoryEvent, SemanticHistoryIndex, SemanticHistoryKind, SemanticHistoryOrigin,
    SemanticHistoryStore,
};

/// A bounded question about one branch's history.
///
/// Every axis is optional and every axis is closed. An axis this boundary does not know is refused
/// rather than ignored, because an ignored filter silently answers a different question than the one
/// that was asked.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryQuery {
    /// The branch to read.
    pub branch_id: String,
    /// Restrict to one kind.
    pub kind: Option<SemanticHistoryKind>,
    /// Restrict to one origin.
    pub origin: Option<SemanticHistoryOrigin>,
    /// Restrict to events whose subject is this live instance.
    pub subject_id: Option<String>,
    /// Restrict to one episode.
    pub episode_id: Option<String>,
    /// Restrict to sequences at or after this number.
    pub from_sequence: Option<u64>,
    /// Restrict to sequences at or before this number.
    pub to_sequence: Option<u64>,
    /// Maximum events to return.
    pub limit: usize,
}

impl SemanticHistoryQuery {
    /// A query for one branch with no filter beyond the page bound.
    #[must_use]
    pub fn branch(branch_id: &str, limit: usize) -> Self {
        Self {
            branch_id: branch_id.to_owned(),
            kind: None,
            origin: None,
            subject_id: None,
            episode_id: None,
            from_sequence: None,
            to_sequence: None,
            limit,
        }
    }

    /// Validates the bounds, including that the window is coherent.
    pub fn validate(&self) -> Result<(), Error> {
        if self.limit == 0 || self.limit > MAX_HISTORY_PAGE {
            return Err(Error::Bounds);
        }
        if let (Some(from), Some(to)) = (self.from_sequence, self.to_sequence)
            && from > to
        {
            return Err(Error::Bounds);
        }
        if let Some(subject) = &self.subject_id {
            super::validate_history_identity(subject, "query.subject_id")?;
        }
        if let Some(episode) = &self.episode_id {
            super::validate_history_identity(episode, "query.episode_id")?;
        }
        Ok(())
    }

    /// The digest that binds a cursor to this exact question.
    pub fn digest(&self) -> Result<String, Error> {
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Corrupt)?;
        Ok(super::history_digest(&bytes))
    }

    fn admits(&self, event: &SemanticHistoryEvent) -> bool {
        if self.kind.is_some_and(|kind| kind != event.input.kind) {
            return false;
        }
        if self
            .origin
            .is_some_and(|origin| origin != event.input.origin)
        {
            return false;
        }
        if let Some(subject) = &self.subject_id {
            let matches = event
                .input
                .subject
                .as_ref()
                .is_some_and(|candidate| &candidate.identity == subject);
            if !matches {
                return false;
            }
        }
        if self
            .episode_id
            .as_ref()
            .is_some_and(|episode| episode != &event.input.episode_id)
        {
            return false;
        }
        if self
            .from_sequence
            .is_some_and(|from| event.input.sequence < from)
        {
            return false;
        }
        if self.to_sequence.is_some_and(|to| event.input.sequence > to) {
            return false;
        }
        true
    }
}

/// One bounded page of history, with the coverage the page cannot show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryPage {
    /// The events in this page, in sequence order.
    pub events: Vec<SemanticHistoryEvent>,
    /// A continuation when more matching events exist beyond this page.
    pub continuation: Option<SemanticHistoryContinuation>,
    /// Declared gaps intersecting the requested window, so a short page is never read as a quiet run.
    pub gaps: Vec<SemanticHistoryCoverageStatus>,
    /// Whether the requested window reaches before capture began.
    pub before_capture: bool,
}

/// A single-use continuation bound to one query and one generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryContinuation {
    /// The cursor to resume from.
    pub cursor: SemanticHistoryCursor,
}

/// A bounded summary of one branch's history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistorySummary {
    /// The branch summarized.
    pub branch_id: String,
    /// Total events recorded on the branch.
    pub total: usize,
    /// Events observed at the host boundary.
    pub captured: usize,
    /// Events recorded as dropped or unsupported.
    pub gaps: usize,
    /// Lowest sequence recorded.
    pub first_sequence: Option<u64>,
    /// Highest sequence recorded.
    pub last_sequence: Option<u64>,
}

/// A reader bound to one store, one branch scope and one generation.
///
/// The reader holds a borrow of the store rather than a copy of it, so a page can never be served
/// from a history that has since advanced, and a continuation can never be replayed against a
/// different question.
pub struct SemanticHistoryReader<'a> {
    store: &'a SemanticHistoryStore,
    index: SemanticHistoryIndex,
    generation: u64,
}

impl<'a> SemanticHistoryReader<'a> {
    /// Opens a reader over one branch.
    pub fn open(store: &'a SemanticHistoryStore, branch_id: &str) -> Result<Self, Error> {
        let index = store.index(branch_id)?;
        let generation = index.generation();
        Ok(Self {
            store,
            index,
            generation,
        })
    }

    /// The generation this reader serves.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Serves one bounded page.
    ///
    /// A continuation is returned only when more matching events exist, and it is bound to this
    /// query's digest and this reader's generation. A page that does not cover every match therefore
    /// always hands back something a caller can actually use.
    pub fn page(
        &self,
        query: &SemanticHistoryQuery,
        continuation: Option<&SemanticHistoryContinuation>,
    ) -> Result<SemanticHistoryPage, Error> {
        query.validate()?;
        let digest = query.digest()?;
        let start_after = match continuation {
            Some(continuation) => {
                if continuation.cursor.query_digest != digest
                    || continuation.cursor.generation != self.generation
                {
                    // A cursor from another question or another generation would answer a different
                    // query than the one asked, so it is refused rather than coerced.
                    return Err(Error::Continuation);
                }
                Some(continuation.cursor.next_sequence)
            }
            None => None,
        };
        let events = self.store.events(&query.branch_id)?;
        let mut matched: Vec<&SemanticHistoryEvent> = Vec::new();
        for ordinal in self.index.all() {
            let event = &events[*ordinal];
            if !query.admits(event) {
                continue;
            }
            if start_after.is_some_and(|after| event.input.sequence < after) {
                continue;
            }
            matched.push(event);
        }
        let more = matched.len() > query.limit;
        let page_events: Vec<SemanticHistoryEvent> = matched
            .iter()
            .take(query.limit)
            .map(|event| (*event).clone())
            .collect();
        let continuation = if more {
            let last = page_events
                .last()
                .ok_or(Error::Continuation)?
                .input
                .sequence;
            Some(SemanticHistoryContinuation {
                cursor: self.index.cursor(&digest, last + 1)?,
            })
        } else {
            None
        };
        let window = self.store.capture_window();
        let from = query.from_sequence.unwrap_or(window.capture_start);
        let to = query.to_sequence.unwrap_or_else(|| {
            page_events
                .last()
                .map_or(from, |event| event.input.sequence)
        });
        let gaps = window
            .intervals
            .iter()
            .filter(|interval| interval.to_sequence >= from && interval.from_sequence <= to)
            .map(|interval| interval.status)
            .collect();
        let before_capture = query
            .from_sequence
            .is_some_and(|sequence| window.is_before_capture(sequence));
        Ok(SemanticHistoryPage {
            events: page_events,
            continuation,
            gaps,
            before_capture,
        })
    }

    /// Summarizes the branch, counting gaps separately from observations.
    pub fn summary(&self, branch_id: &str) -> Result<SemanticHistorySummary, Error> {
        let events = self.store.events(branch_id)?;
        let captured = events
            .iter()
            .filter(|event| event.input.coverage.status.is_observed())
            .count();
        let first_sequence = events.iter().map(|event| event.input.sequence).min();
        let last_sequence = events.iter().map(|event| event.input.sequence).max();
        Ok(SemanticHistorySummary {
            branch_id: branch_id.to_owned(),
            total: events.len(),
            captured,
            gaps: events.len() - captured,
            first_sequence,
            last_sequence,
        })
    }
}
