// SPDX-License-Identifier: MIT

//! Why-changed traversal: bounded causal explanation from stated links only.

use super::{
    Error, MAX_HISTORY_TRAVERSAL_DEPTH, MAX_HISTORY_TRAVERSAL_VISITS, SemanticHistoryEvent,
    SemanticHistoryKind, SemanticHistoryStore, SemanticHistoryValue,
};
use std::collections::BTreeSet;

/// One edge of a causal explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryLink {
    /// The causing event.
    pub from_event_id: String,
    /// The caused event.
    pub to_event_id: String,
    /// How many stated edges separate the two.
    pub depth: usize,
}

/// Bounds one traversal may consume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticHistoryTraversalLimits {
    /// Maximum stated edges to walk.
    pub max_depth: usize,
    /// Maximum events to visit.
    pub max_visits: usize,
}

impl SemanticHistoryTraversalLimits {
    /// The default bounds.
    #[must_use]
    pub const fn bounded() -> Self {
        Self {
            max_depth: MAX_HISTORY_TRAVERSAL_DEPTH,
            max_visits: MAX_HISTORY_TRAVERSAL_VISITS,
        }
    }

    /// Validates that the limits are non-zero and inside the hard bounds.
    pub fn validate(&self) -> Result<(), Error> {
        if self.max_depth == 0
            || self.max_depth > MAX_HISTORY_TRAVERSAL_DEPTH
            || self.max_visits == 0
            || self.max_visits > MAX_HISTORY_TRAVERSAL_VISITS
        {
            return Err(Error::Bounds);
        }
        Ok(())
    }
}

/// The result of one why-changed traversal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryTraversal {
    /// The event the question started from.
    pub root_event_id: String,
    /// The stated links, nearest first.
    pub links: Vec<SemanticHistoryLink>,
    /// Every event visited, in the order they were reached.
    pub visited: Vec<String>,
    /// Whether the walk stopped at a bound rather than at a root.
    pub truncated: bool,
    /// Whether the root's own cause is unknown to this boundary.
    pub root_cause_unstated: bool,
}

/// A why-changed explanation for one observed change.
///
/// The explanation is built only from links the host stated. Where a link is absent the answer says
/// so, because inferring a cause from a difference between two snapshots would present a guess as a
/// fact — which is exactly what a why-changed query is asked to avoid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryExplanation {
    /// The event whose value changed.
    pub event_id: String,
    /// The value the event states, if it states one.
    pub value: Option<SemanticHistoryValue>,
    /// The kind of change.
    pub kind: SemanticHistoryKind,
    /// The stated causal chain leading to this event.
    pub traversal: SemanticHistoryTraversal,
}

impl SemanticHistoryExplanation {
    /// Returns whether the boundary can explain why this value changed.
    #[must_use]
    pub fn is_explained(&self) -> bool {
        !self.traversal.links.is_empty()
    }
}

impl SemanticHistoryStore {
    /// Explains one event by walking its stated causal links backwards.
    ///
    /// The walk is bounded by depth and by visits, and it refuses a cycle rather than looping. A
    /// traversal that stops at a bound reports `truncated` so a caller cannot mistake a bounded
    /// answer for a complete one.
    pub fn explain(
        &self,
        branch_id: &str,
        event_id: &str,
        limits: SemanticHistoryTraversalLimits,
    ) -> Result<SemanticHistoryExplanation, Error> {
        limits.validate()?;
        let root = self.event(branch_id, event_id)?.clone();
        let mut links: Vec<SemanticHistoryLink> = Vec::new();
        let mut visited: Vec<String> = vec![root.input.event_id.clone()];
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(root.input.event_id.clone());
        let mut frontier: Vec<(String, usize)> = vec![(root.input.event_id.clone(), 0)];
        let mut truncated = false;
        while let Some((current_id, depth)) = frontier.pop() {
            let current = self.event(branch_id, &current_id)?;
            let Some(parent) = current.causal_parent.stated_event_id() else {
                continue;
            };
            if depth >= limits.max_depth {
                // There is a stated cause beyond this bound; say so rather than ending the walk.
                truncated = true;
                continue;
            }
            if visited.len() >= limits.max_visits {
                truncated = true;
                continue;
            }
            if !seen.insert(parent.to_owned()) {
                // A stated link that returns to an already-visited event is a cycle.
                return Err(Error::Cycle);
            }
            let parent_event: &SemanticHistoryEvent = self.event(branch_id, parent)?;
            links.push(SemanticHistoryLink {
                from_event_id: parent.to_owned(),
                to_event_id: current_id.clone(),
                depth: depth + 1,
            });
            visited.push(parent_event.input.event_id.clone());
            frontier.push((parent.to_owned(), depth + 1));
        }
        Ok(SemanticHistoryExplanation {
            event_id: root.input.event_id.clone(),
            value: root.input.value.clone(),
            kind: root.input.kind,
            traversal: SemanticHistoryTraversal {
                root_event_id: root.input.event_id.clone(),
                root_cause_unstated: !root.causal_parent.is_stated(),
                links,
                visited,
                truncated,
            },
        })
    }
}
