// SPDX-License-Identifier: MIT

//! Indexes over one branch's history, used to answer bounded queries.

use super::{
    Error, SemanticHistoryEvent, SemanticHistoryKind, SemanticHistoryNamespace,
    SemanticHistoryOrigin,
};
use std::collections::{BTreeMap, BTreeSet};

/// A resumable position inside one ordered query result.
///
/// A cursor is bound to the query it was issued for and to the store generation it was taken in, so
/// a cursor cannot be replayed against a different question or a history that has since advanced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryCursor {
    /// The query digest this cursor belongs to.
    pub query_digest: String,
    /// The generation the cursor was taken in.
    pub generation: u64,
    /// The next sequence number to serve.
    pub next_sequence: u64,
}

/// One branch's events indexed by the fields a bounded query may filter on.
pub struct SemanticHistoryIndex {
    branch_id: String,
    generation: u64,
    by_sequence: Vec<usize>,
    by_kind: BTreeMap<SemanticHistoryKind, Vec<usize>>,
    by_origin: BTreeMap<SemanticHistoryOrigin, Vec<usize>>,
    by_subject: BTreeMap<String, Vec<usize>>,
    by_episode: BTreeMap<u64, Vec<usize>>,
}

impl SemanticHistoryIndex {
    /// Builds the index over one branch's stored events.
    #[must_use]
    pub fn build(branch_id: &str, events: &[SemanticHistoryEvent]) -> Self {
        let mut by_sequence: Vec<usize> = (0..events.len()).collect();
        by_sequence.sort_by_key(|index| events[*index].input.sequence);
        let mut by_kind: BTreeMap<SemanticHistoryKind, Vec<usize>> = BTreeMap::new();
        let mut by_origin: BTreeMap<SemanticHistoryOrigin, Vec<usize>> = BTreeMap::new();
        let mut by_subject: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut by_episode: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (index, event) in events.iter().enumerate() {
            by_kind.entry(event.input.kind).or_default().push(index);
            by_origin.entry(event.input.origin).or_default().push(index);
            // An event whose actor and target are the same live instance appears once under that
            // identity, so a filter cannot return the same event twice.
            let mut identities: BTreeSet<&str> = BTreeSet::new();
            for subject in &event.input.subjects {
                if subject.namespace == SemanticHistoryNamespace::LiveInstance {
                    identities.insert(&subject.identity);
                }
            }
            for identity in identities {
                by_subject
                    .entry(identity.to_owned())
                    .or_default()
                    .push(index);
            }
            by_episode
                .entry(event.input.episode)
                .or_default()
                .push(index);
        }
        Self {
            branch_id: branch_id.to_owned(),
            generation: events.len() as u64,
            by_sequence,
            by_kind,
            by_origin,
            by_subject,
            by_episode,
        }
    }

    /// The branch this index covers.
    #[must_use]
    pub fn branch_id(&self) -> &str {
        &self.branch_id
    }

    /// The generation this index was built at.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Every event ordinal, in sequence order.
    #[must_use]
    pub fn all(&self) -> &[usize] {
        &self.by_sequence
    }

    /// Ordinals for one kind.
    #[must_use]
    pub fn kind(&self, kind: SemanticHistoryKind) -> &[usize] {
        self.by_kind.get(&kind).map_or(&[], Vec::as_slice)
    }

    /// Ordinals for one origin.
    #[must_use]
    pub fn origin(&self, origin: SemanticHistoryOrigin) -> &[usize] {
        self.by_origin.get(&origin).map_or(&[], Vec::as_slice)
    }

    /// Ordinals whose subject is one live instance.
    #[must_use]
    pub fn subject(&self, identity: &str) -> &[usize] {
        self.by_subject.get(identity).map_or(&[], Vec::as_slice)
    }

    /// Ordinals for one episode.
    #[must_use]
    pub fn episode(&self, episode: u64) -> &[usize] {
        self.by_episode.get(&episode).map_or(&[], Vec::as_slice)
    }

    /// Mints a cursor for a query at this generation.
    pub fn cursor(
        &self,
        query_digest: &str,
        next_sequence: u64,
    ) -> Result<SemanticHistoryCursor, Error> {
        super::validate_history_identity(query_digest, "cursor.query_digest")?;
        Ok(SemanticHistoryCursor {
            query_digest: query_digest.to_owned(),
            generation: self.generation,
            next_sequence,
        })
    }
}
