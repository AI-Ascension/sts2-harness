// SPDX-License-Identifier: MIT

//! Durable append, ordering, retention and idempotency for one run's semantic history.

use super::{
    Error, MAX_HISTORY_BRANCH_DEPTH, MAX_HISTORY_EVENT_BYTES, MAX_HISTORY_EVENTS,
    SEMANTIC_HISTORY_SCHEMA, SemanticHistoryBinding, SemanticHistoryCaptureWindow,
    SemanticHistoryCausalParent, SemanticHistoryCoverageStatus, SemanticHistoryEvent,
    SemanticHistoryEventInput, SemanticHistoryIndex, SemanticHistoryLineage, SemanticHistoryOrigin,
    history_digest, validation,
};
use std::collections::BTreeMap;

#[path = "semantic_history_store_retention.rs"]
mod retention;

/// The outcome of one append attempt.
///
/// A replay is a distinct outcome from a new append: the caller can tell that nothing was written
/// because the event was already recorded, rather than assuming a second event exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticHistoryAppend {
    /// The event was newly recorded.
    Recorded,
    /// The event was already recorded with identical content, so nothing was written.
    Replayed,
}

/// How retention treats one stored event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticHistoryRetention {
    /// The event is retained in full.
    Retained,
    /// The event is retained, but its value payload was redacted by privacy policy.
    Redacted,
}

/// One run's durable semantic history.
///
/// The store is append-only in the sense that matters here: an event that has been recorded is never
/// rewritten into a different event. Re-appending the same identity with identical content replays,
/// and re-appending it with different content is refused as a conflict rather than accepted as a
/// correction, because a history that can be edited in place cannot be trusted to explain a run.
pub struct SemanticHistoryStore {
    binding: SemanticHistoryBinding,
    lineage: Vec<SemanticHistoryLineage>,
    branches: BTreeMap<String, BranchHistory>,
    window: SemanticHistoryCaptureWindow,
}

struct BranchHistory {
    events: Vec<SemanticHistoryEvent>,
    by_id: BTreeMap<String, usize>,
    last_sequence: Option<u64>,
}

impl SemanticHistoryStore {
    /// Opens a store for one owner scope, with the run root as the only branch.
    pub fn open(
        binding: SemanticHistoryBinding,
        root_branch_id: &str,
        window: SemanticHistoryCaptureWindow,
    ) -> Result<Self, Error> {
        binding.validate()?;
        window.validate()?;
        let root = SemanticHistoryLineage {
            branch_id: root_branch_id.to_owned(),
            parent_branch_id: None,
            fork_sequence: 0,
            authority_epoch: binding.authority_epoch,
        };
        root.validate()?;
        let mut branches = BTreeMap::new();
        branches.insert(root_branch_id.to_owned(), BranchHistory::new());
        Ok(Self {
            binding,
            lineage: vec![root],
            branches,
            window,
        })
    }

    /// The owner scope this store serves.
    #[must_use]
    pub fn binding(&self) -> &SemanticHistoryBinding {
        &self.binding
    }

    /// The capture window, including every declared gap.
    #[must_use]
    pub fn capture_window(&self) -> &SemanticHistoryCaptureWindow {
        &self.window
    }

    /// Every branch edge recorded so far.
    #[must_use]
    pub fn lineage(&self) -> &[SemanticHistoryLineage] {
        &self.lineage
    }

    /// Registers a child branch forked from an existing one.
    ///
    /// A child starts with no events of its own. It does not inherit its parent's events: ancestry is
    /// followed through the lineage, so a fork cannot be mistaken for a duplicate of its parent.
    pub fn fork(
        &mut self,
        branch_id: &str,
        parent_branch_id: &str,
        fork_sequence: u64,
    ) -> Result<(), Error> {
        if !self.branches.contains_key(parent_branch_id) {
            return Err(Error::Branch);
        }
        if self.branches.contains_key(branch_id) {
            return Err(Error::Branch);
        }
        let edge = SemanticHistoryLineage {
            branch_id: branch_id.to_owned(),
            parent_branch_id: Some(parent_branch_id.to_owned()),
            fork_sequence,
            authority_epoch: self.binding.authority_epoch,
        };
        edge.validate()?;
        self.assert_lineage_depth(parent_branch_id)?;
        self.lineage.push(edge);
        self.branches
            .insert(branch_id.to_owned(), BranchHistory::new());
        Ok(())
    }

    /// Counts the ancestry a child of `parent_branch_id` would have, and refuses one deeper than
    /// the bound.
    ///
    /// The walk starts at the parent because the child's own edge has not been recorded yet; it also
    /// refuses a parent chain that returns to a branch it has already passed.
    fn assert_lineage_depth(&self, parent_branch_id: &str) -> Result<(), Error> {
        let mut depth = 1_usize;
        let mut current = Some(parent_branch_id.to_owned());
        let mut seen = std::collections::BTreeSet::new();
        while let Some(branch) = current {
            if !seen.insert(branch.clone()) {
                return Err(Error::Lineage);
            }
            depth += 1;
            if depth > MAX_HISTORY_BRANCH_DEPTH {
                return Err(Error::Lineage);
            }
            current = self
                .lineage
                .iter()
                .find(|edge| edge.branch_id == branch)
                .and_then(|edge| edge.parent_branch_id.clone());
        }
        Ok(())
    }

    /// Advances the authority epoch.
    ///
    /// Advancing the epoch is the one operation that resets sequence expectations: a new epoch may
    /// restart host sequencing, and treating a restarted sequence as a backwards move would refuse
    /// an honest capture. Everything already stored keeps the epoch it was recorded under.
    pub fn advance_epoch(&mut self, epoch: u64) -> Result<(), Error> {
        if epoch <= self.binding.authority_epoch {
            return Err(Error::Epoch);
        }
        self.binding.authority_epoch = epoch;
        for branch in self.branches.values_mut() {
            branch.last_sequence = None;
        }
        Ok(())
    }

    /// Appends one event, or replays an identical one.
    ///
    /// Idempotency is by event identity inside one branch. The same identity with identical content
    /// writes nothing; the same identity with different content is refused, so a replay or a rejoin
    /// can never turn one event into two.
    pub fn append(
        &mut self,
        branch_id: &str,
        input: SemanticHistoryEventInput,
        causal_parent: SemanticHistoryCausalParent,
    ) -> Result<SemanticHistoryAppend, Error> {
        causal_parent.validate()?;
        validation::validate_input(&input, &causal_parent, branch_id, &self.binding.run_id)?;
        if causal_parent.is_stated() && !input.origin.admits_stated_parent() {
            // An imported event's causality was settled when it was captured.
            return Err(Error::ImportedStatesParent);
        }
        if input.authority_epoch != self.binding.authority_epoch {
            return Err(Error::Epoch);
        }
        if self.window.is_before_capture(input.sequence) {
            return Err(Error::Coverage);
        }
        // A captured event may not sit inside a declared gap: that would close the gap with a value
        // the capture explicitly said it could not observe.
        if let Some(gap) = self.window.gap_at(input.sequence) {
            if input.coverage.status.is_observed() {
                return Err(Error::Coverage);
            }
            if gap.status != input.coverage.status {
                return Err(Error::Coverage);
            }
        }
        let digest = self.digest_of(&input, &causal_parent)?;
        let branch = self.branches.get_mut(branch_id).ok_or(Error::Branch)?;
        if let Some(index) = branch.by_id.get(&input.event_id).copied() {
            let existing = &branch.events[index];
            if existing.content_digest == digest {
                return Ok(SemanticHistoryAppend::Replayed);
            }
            // The same identity now describes a different event.
            return Err(Error::MixedGeneration);
        }
        if branch.events.len() >= MAX_HISTORY_EVENTS {
            return Err(Error::Capacity);
        }
        if let Some(last) = branch.last_sequence {
            if input.sequence <= last {
                // A sequence that does not advance would make order ambiguous.
                return Err(Error::Sequence);
            }
            if input.sequence > last + 1 {
                // A jump must be declared as coverage; it is never closed silently.
                let declared = self
                    .window
                    .gap_at(last + 1)
                    .is_some_and(|gap| gap.to_sequence >= input.sequence - 1);
                if !declared {
                    return Err(Error::Sequence);
                }
            }
        }
        if let Some(parent) = causal_parent.stated_event_id() {
            let parent_index = branch.by_id.get(parent).copied().ok_or(Error::Causality)?;
            if branch.events[parent_index].input.sequence >= input.sequence {
                return Err(Error::ParentNotPreceding);
            }
            if branch.events[parent_index].input.authority_epoch != input.authority_epoch {
                return Err(Error::Causality);
            }
        }
        let event = SemanticHistoryEvent {
            schema: SEMANTIC_HISTORY_SCHEMA.to_owned(),
            input,
            branch_id: branch_id.to_owned(),
            causal_parent,
            content_digest: digest,
        };
        let index = branch.events.len();
        branch.last_sequence = Some(event.input.sequence);
        branch.by_id.insert(event.input.event_id.clone(), index);
        branch.events.push(event);
        Ok(SemanticHistoryAppend::Recorded)
    }

    fn digest_of(
        &self,
        input: &SemanticHistoryEventInput,
        causal_parent: &SemanticHistoryCausalParent,
    ) -> Result<String, Error> {
        let bytes = serde_json::to_vec(&(input, causal_parent)).map_err(|_| Error::Corrupt)?;
        if bytes.len() > MAX_HISTORY_EVENT_BYTES {
            return Err(Error::Bounds);
        }
        Ok(history_digest(&bytes))
    }
}
