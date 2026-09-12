// SPDX-License-Identifier: MIT

//! Immutable occurrence lineage for checkpoint experiments.
//!
//! An occurrence is one recorded visit to a gameplay state. Two occurrences may share an exact
//! state digest, but they remain distinct nodes with distinct parents, so content deduplication
//! never merges histories. Parents must be inserted before their children, which makes the graph
//! acyclic by construction; a repeated state can still be reached through different parents.

use std::collections::BTreeMap;

use crate::execution::{ExactCheckpointId, ExactStateDigest};

use super::MAX_TRANSITION_LABEL_BYTES;

/// Maximum occurrences retained in one graph.
pub const MAX_OCCURRENCES: usize = 100_000;

/// Rejection reasons for occurrence lineage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineageError {
    /// An identifier is empty, too long, or outside the accepted alphabet.
    InvalidIdentifier,
    /// A label field is empty, too long, or contains a NUL separator.
    InvalidRecord,
    /// The occurrence identifier is already present.
    DuplicateOccurrence,
    /// The referenced parent occurrence was never inserted.
    UnknownParent,
    /// The occurrence identifier is unknown.
    UnknownOccurrence,
    /// The graph reached its bounded capacity.
    Capacity,
}

/// A stable identifier for one occurrence in a lineage graph.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OccurrenceId(String);

impl OccurrenceId {
    /// Parses an occurrence identifier, rejecting empty or oversized values.
    pub fn parse(value: &str) -> Result<Self, LineageError> {
        let accepted = !value.is_empty()
            && value.len() <= MAX_TRANSITION_LABEL_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-')
            });
        if !accepted {
            return Err(LineageError::InvalidIdentifier);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the serialized identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One recorded occurrence and its immutable parentage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrenceRecord {
    /// This occurrence's identifier.
    pub occurrence_id: OccurrenceId,
    /// Parent occurrence, absent only at a root.
    pub parent: Option<OccurrenceId>,
    /// Immutable checkpoint restored to reach this occurrence, when one was used.
    pub parent_checkpoint: Option<ExactCheckpointId>,
    /// Logical action that produced this occurrence, absent at a root.
    pub action_key: Option<String>,
    /// Exact state identity observed at this occurrence.
    pub state_digest: ExactStateDigest,
    /// Experiment or trial identifier, kept outside gameplay identity.
    pub experiment_id: Option<String>,
}

impl OccurrenceRecord {
    fn validate(&self) -> Result<(), LineageError> {
        if self.parent.is_some() != self.action_key.is_some() {
            return Err(LineageError::InvalidRecord);
        }
        for label in [self.action_key.as_deref(), self.experiment_id.as_deref()]
            .into_iter()
            .flatten()
        {
            if label.is_empty() || label.len() > MAX_TRANSITION_LABEL_BYTES || label.contains('\0')
            {
                return Err(LineageError::InvalidRecord);
            }
        }
        if self.parent.is_none() && self.parent_checkpoint.is_some() {
            return Err(LineageError::InvalidRecord);
        }
        Ok(())
    }
}

/// An acyclic occurrence graph built by inserting parents before children.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OccurrenceGraph {
    records: Vec<OccurrenceRecord>,
    index: BTreeMap<String, usize>,
}

impl OccurrenceGraph {
    /// Creates an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of recorded occurrences.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Reports whether no occurrence has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Inserts one occurrence, requiring its parent to already exist.
    pub fn insert(&mut self, record: OccurrenceRecord) -> Result<(), LineageError> {
        record.validate()?;
        if self.records.len() >= MAX_OCCURRENCES {
            return Err(LineageError::Capacity);
        }
        if self.index.contains_key(record.occurrence_id.as_str()) {
            return Err(LineageError::DuplicateOccurrence);
        }
        if let Some(parent) = &record.parent
            && !self.index.contains_key(parent.as_str())
        {
            return Err(LineageError::UnknownParent);
        }
        self.index
            .insert(record.occurrence_id.as_str().to_owned(), self.records.len());
        self.records.push(record);
        Ok(())
    }

    /// Returns the ancestry of an occurrence, root first.
    pub fn ancestors(&self, id: &OccurrenceId) -> Result<Vec<OccurrenceId>, LineageError> {
        let mut chain = Vec::new();
        let mut current = Some(id.clone());
        while let Some(step) = current {
            let record = self.record(&step)?;
            chain.push(step);
            current = record.parent.clone();
        }
        chain.reverse();
        Ok(chain)
    }

    /// Returns the root occurrence of an occurrence's ancestry.
    pub fn root(&self, id: &OccurrenceId) -> Result<OccurrenceId, LineageError> {
        let ancestry = self.ancestors(id)?;
        ancestry
            .first()
            .cloned()
            .ok_or(LineageError::UnknownOccurrence)
    }

    /// Returns every occurrence observed with the given exact state identity.
    #[must_use]
    pub fn occurrences_with_state(&self, state_digest: &ExactStateDigest) -> Vec<OccurrenceId> {
        self.records
            .iter()
            .filter(|record| &record.state_digest == state_digest)
            .map(|record| record.occurrence_id.clone())
            .collect()
    }

    /// Groups occurrence identifiers by ancestry root, which keeps duplicates in one split.
    pub fn group_by_root(&self) -> Result<BTreeMap<OccurrenceId, Vec<OccurrenceId>>, LineageError> {
        let mut groups: BTreeMap<OccurrenceId, Vec<OccurrenceId>> = BTreeMap::new();
        for record in &self.records {
            let root = self.root(&record.occurrence_id)?;
            groups
                .entry(root)
                .or_default()
                .push(record.occurrence_id.clone());
        }
        Ok(groups)
    }

    fn record(&self, id: &OccurrenceId) -> Result<&OccurrenceRecord, LineageError> {
        self.index
            .get(id.as_str())
            .and_then(|position| self.records.get(*position))
            .ok_or(LineageError::UnknownOccurrence)
    }
}
