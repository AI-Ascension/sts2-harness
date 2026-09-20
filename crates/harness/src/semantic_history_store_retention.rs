// SPDX-License-Identifier: MIT

//! Read, index, coverage and retention surface over one run's stored semantic history.
//!
//! Everything here reads what the store already holds, except retention, which redacts one value
//! payload in place: it keeps the event, its coverage and its causal link, and reports the value as
//! unavailable rather than removing the event or leaving a zero behind.

use super::*;
use crate::semantic_history::SemanticHistoryValue;

impl SemanticHistoryStore {
    /// Every event recorded on one branch, in stored order.
    pub fn events(&self, branch_id: &str) -> Result<&[SemanticHistoryEvent], Error> {
        Ok(&self.branches.get(branch_id).ok_or(Error::Branch)?.events)
    }

    /// One event by identity, if it was recorded on that branch.
    pub fn event(&self, branch_id: &str, event_id: &str) -> Result<&SemanticHistoryEvent, Error> {
        let branch = self.branches.get(branch_id).ok_or(Error::Branch)?;
        let index = branch.by_id.get(event_id).ok_or(Error::Causality)?;
        Ok(&branch.events[*index])
    }

    /// Applies retention to one event's value payload.
    ///
    /// Redaction keeps the event, its coverage and its causal link, and replaces the value with an
    /// explicit `Unavailable`. It never deletes the event and never leaves a zero behind, because
    /// either would misreport the run: a removed event looks like it never happened, and a zeroed
    /// value looks like it happened differently.
    pub fn apply_retention(
        &mut self,
        branch_id: &str,
        event_id: &str,
        redact: bool,
    ) -> Result<SemanticHistoryRetention, Error> {
        let branch = self.branches.get_mut(branch_id).ok_or(Error::Branch)?;
        let index = *branch.by_id.get(event_id).ok_or(Error::Causality)?;
        if !redact {
            return Ok(SemanticHistoryRetention::Retained);
        }
        let event = &mut branch.events[index];
        let digest = event.content_digest.clone();
        event.input.value = Some(SemanticHistoryValue::Unavailable {
            reason: "retention".to_owned(),
        });
        // The digest is deliberately left as recorded: it identifies the event's identity, not its
        // current payload, so a later replay of the original content is still recognised.
        debug_assert_eq!(event.content_digest, digest);
        Ok(SemanticHistoryRetention::Redacted)
    }

    /// Builds the query index over one branch.
    pub fn index(&self, branch_id: &str) -> Result<SemanticHistoryIndex, Error> {
        let branch = self.branches.get(branch_id).ok_or(Error::Branch)?;
        Ok(SemanticHistoryIndex::build(branch_id, &branch.events))
    }

    /// Returns the status recorded for a sequence number, if any event states it.
    pub fn coverage_at(
        &self,
        branch_id: &str,
        sequence: u64,
    ) -> Result<Option<SemanticHistoryCoverageStatus>, Error> {
        let branch = self.branches.get(branch_id).ok_or(Error::Branch)?;
        Ok(branch
            .events
            .iter()
            .find(|event| event.input.sequence == sequence)
            .map(|event| event.input.coverage.status))
    }

    /// Returns the number of events recorded on one branch.
    pub fn len(&self, branch_id: &str) -> Result<usize, Error> {
        Ok(self
            .branches
            .get(branch_id)
            .ok_or(Error::Branch)?
            .events
            .len())
    }

    /// Returns whether one branch holds no events.
    pub fn is_empty(&self, branch_id: &str) -> Result<bool, Error> {
        Ok(self.len(branch_id)? == 0)
    }

    /// Returns whether an origin may state a causal parent.
    #[must_use]
    pub const fn origin_admits_parent(origin: SemanticHistoryOrigin) -> bool {
        origin.admits_stated_parent()
    }
}

impl BranchHistory {
    pub(super) fn new() -> Self {
        Self {
            events: Vec::new(),
            by_id: BTreeMap::new(),
            last_sequence: None,
        }
    }
}
