// SPDX-License-Identifier: MIT

//! Per-scope consumption state and the pure-read preview of it (issue #111).

use std::collections::BTreeSet;

use super::lifetime_scope::ContextLifetimeScope;

/// A pure-read view of one scope's remaining applicability and the reasons for it.
///
/// Producing this never consumes, extends, or resurrects anything: it is the shape a preview,
/// reload, or receipt lookup may safely observe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifetimePreview {
    pub scope_id: String,
    pub scope_digest: String,
    pub capacity: u32,
    pub consumed: u32,
    pub remaining: u32,
    /// Invocations admitted but not yet reconciled, ordered.
    pub held: Vec<String>,
    /// Whether the wall-clock ceiling has passed at the observed instant.
    pub expired: bool,
    pub items: Vec<String>,
}

/// The consumption state of one issued scope.
pub(super) struct ScopeState {
    pub(super) scope: ContextLifetimeScope,
    pub(super) digest: String,
    /// Distinct logical invocations in admission order; a retry never appends.
    pub(super) admitted: Vec<String>,
    /// Invocations whose dispatch outcome is still unknown.
    pub(super) held: BTreeSet<String>,
    /// Invocations reconciled as never dispatched; these release their slot.
    pub(super) released: BTreeSet<String>,
}

impl ScopeState {
    /// Slots actually consumed: every admitted invocation except the released ones.
    ///
    /// A held invocation stays counted, because a dispatch that may have happened must not be
    /// handed back before reconciliation says it did not.
    pub(super) fn consumed(&self) -> u32 {
        let released = self
            .admitted
            .iter()
            .filter(|invocation| self.released.contains(*invocation))
            .count();
        u32::try_from(self.admitted.len().saturating_sub(released)).unwrap_or(u32::MAX)
    }

    pub(super) fn manifest_index(&self, invocation_id: &str) -> Option<usize> {
        self.admitted
            .iter()
            .position(|candidate| candidate == invocation_id)
    }

    pub(super) fn preview(&self, now: u64) -> LifetimePreview {
        let consumed = self.consumed();
        LifetimePreview {
            scope_id: self.scope.scope_id.clone(),
            scope_digest: self.digest.clone(),
            capacity: self.scope.applicability.capacity(),
            consumed,
            remaining: self.scope.applicability.capacity().saturating_sub(consumed),
            held: self.held.iter().cloned().collect(),
            expired: now >= self.scope.ceiling,
            items: self.scope.items.clone(),
        }
    }
}
