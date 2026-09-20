// SPDX-License-Identifier: MIT

//! The bounded "why did this change" traversal.
//!
//! Causality here is only ever what the producer stated. The traversal walks stated parents
//! backwards from a starting event, stops at a disclosed `NotStated` parent, and refuses rather
//! than truncates when it would exceed its bounds or revisit an event.

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::record::SemanticEventRecord;
use super::scope::{SEMANTIC_MAX_CAUSAL_DEPTH, SEMANTIC_MAX_CAUSAL_VISITS};

/// One visited ancestor in a causal traversal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticCausalVisit {
    /// The ancestor's event identity.
    pub event_id: String,
    /// How many stated parent edges from the starting event.
    pub depth: usize,
}

/// The result of walking stated parents backwards from one event.
///
/// `truncated` is always `false`: a traversal that cannot finish refuses instead of returning a
/// partial chain a consumer might read as the whole cause.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticCausalTraversal {
    /// The event the traversal started from.
    pub origin_event_id: String,
    /// Ancestors in order from the starting event outwards.
    pub visits: Vec<SemanticCausalVisit>,
    /// Whether the chain ended because an event disclosed that it states no parent.
    pub ends_at_disclosure: bool,
    /// Always `false`; a bounded traversal refuses rather than truncating.
    pub truncated: bool,
}

impl SemanticCausalTraversal {
    /// Returns the ancestor identities in visitation order.
    #[must_use]
    pub fn ancestor_ids(&self) -> Vec<&str> {
        self.visits
            .iter()
            .map(|visit| visit.event_id.as_str())
            .collect()
    }
}

/// Walks stated causal parents backwards from `origin`.
///
/// The walk visits each event at most once and stops at the first record that discloses no parent.
/// A chain longer than [`SEMANTIC_MAX_CAUSAL_DEPTH`], or one that would visit more than
/// [`SEMANTIC_MAX_CAUSAL_VISITS`] ancestors, is refused; it is never silently cut short.
pub fn traverse_causes(
    records: &[SemanticEventRecord],
    origin_event_id: &str,
) -> SemanticHistoryResult<SemanticCausalTraversal> {
    let index = records
        .iter()
        .map(|record| (record.event_id(), record))
        .collect::<std::collections::BTreeMap<_, _>>();
    if !index.contains_key(origin_event_id) {
        return Err(SemanticHistoryError::about(
            Refusal::StatedParentUnknown,
            origin_event_id,
        ));
    }
    let mut visits = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    let mut current = origin_event_id;
    let mut depth = 0usize;
    loop {
        let record = index
            .get(current)
            .copied()
            .ok_or_else(|| SemanticHistoryError::about(Refusal::StatedParentUnknown, current))?;
        let Some(parent) = record.event.causal_parent.as_ref() else {
            return Ok(SemanticCausalTraversal {
                origin_event_id: origin_event_id.to_owned(),
                visits,
                ends_at_disclosure: true,
                truncated: false,
            });
        };
        let Some(parent_id) = parent.stated_parent() else {
            return Ok(SemanticCausalTraversal {
                origin_event_id: origin_event_id.to_owned(),
                visits,
                ends_at_disclosure: true,
                truncated: false,
            });
        };
        if !visited.insert(parent_id.to_owned()) {
            return Err(SemanticHistoryError::about(Refusal::CausalCycle, parent_id));
        }
        depth += 1;
        if depth > SEMANTIC_MAX_CAUSAL_DEPTH || visits.len() >= SEMANTIC_MAX_CAUSAL_VISITS {
            return Err(SemanticHistoryError::about(
                Refusal::TraversalBound,
                parent_id,
            ));
        }
        visits.push(SemanticCausalVisit {
            event_id: parent_id.to_owned(),
            depth,
        });
        current = parent_id;
    }
}
