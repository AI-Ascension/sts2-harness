// SPDX-License-Identifier: MIT

//! Bounds, catalog binding, declared support state, and the scope a history belongs to.

use serde::{Deserialize, Serialize};

/// Harness-owned producer identity for the retained-history model.
///
/// This is a source identity, not a wire or native ABI version, and it is stored with every
/// history so a record written by another revision is refused rather than reinterpreted.
pub const SEMANTIC_HISTORY_PRODUCER_VERSION: &str = "harness-semantic-history-producer-v1";
/// Maximum bytes accepted for one opaque identity.
pub const SEMANTIC_MAX_IDENTITY_BYTES: usize = 256;
/// Maximum bytes accepted for one owner-defined label.
pub const SEMANTIC_MAX_LABEL_BYTES: usize = 1024;
/// Maximum bytes accepted for the unit of one quantity.
pub const SEMANTIC_MAX_UNIT_BYTES: usize = 64;
/// Maximum events, observed or disclosed, retained for one history.
pub const SEMANTIC_MAX_EVENTS: usize = 4096;
/// Maximum declared coverage intervals in one capture window.
pub const SEMANTIC_MAX_INTERVALS: usize = 64;
/// Maximum aggregate bytes retained for one history.
pub const SEMANTIC_MAX_HISTORY_BYTES: usize = 512 * 1024;
/// Maximum entries returned by one bounded page.
pub const SEMANTIC_MAX_PAGE_ITEMS: usize = 64;
/// Maximum causal ancestors one traversal may visit.
pub const SEMANTIC_MAX_CAUSAL_VISITS: usize = 64;
/// Maximum causal depth one traversal may reach.
pub const SEMANTIC_MAX_CAUSAL_DEPTH: usize = 16;

/// Static catalog identity: content manifest and producer compatibility.
///
/// A history is bound to the manifest it was observed against and the producer that stated it, so a
/// query naming another manifest or another producer is refused rather than answered from this one.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCatalogBinding {
    /// Existing content-manifest invalidation witness digest.
    pub manifest_digest: String,
    /// Exact owner-local producer identity.
    pub producer_version: String,
}

/// The run, branch, episode and epoch one history belongs to.
///
/// Sequence numbers are only monotonic inside one such scope, so the scope travels with every event
/// instead of being assumed to be "the current run".
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEventScope {
    /// Opaque run identity.
    pub run_id: String,
    /// Opaque branch identity inside that run.
    pub branch_id: String,
    /// Episode number inside that branch.
    pub episode: u64,
    /// Monotonic epoch of the observation stream.
    pub epoch: u64,
}

impl SemanticEventScope {
    /// Returns whether two scopes name the same place, ignoring the observation epoch.
    #[must_use]
    pub fn same_place(&self, other: &Self) -> bool {
        self.run_id == other.run_id
            && self.branch_id == other.branch_id
            && self.episode == other.episode
    }

    /// Returns whether two scopes are exactly equal, epoch included.
    #[must_use]
    pub fn same_epoch(&self, other: &Self) -> bool {
        self == other
    }
}

/// The live fence a current-history read holds.
///
/// A history belongs to the run and branch it was observed in, at the epoch the observation was
/// taken. A read that names another place is refused rather than answered with this history.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryFence {
    /// Opaque run identity the observation was taken from.
    pub run_id: String,
    /// Opaque branch identity the observation was taken from.
    pub branch_id: String,
    /// Episode number the observation was taken at.
    pub episode: u64,
    /// Monotonic epoch of that observation stream.
    pub epoch: u64,
}

impl SemanticHistoryFence {
    /// Returns whether a history's scope satisfies this fence exactly.
    #[must_use]
    pub fn admits(&self, scope: &SemanticEventScope) -> bool {
        self.run_id == scope.run_id
            && self.branch_id == scope.branch_id
            && self.episode == scope.episode
            && self.epoch == scope.epoch
    }
}

/// Declared support state for the semantic-history family.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticFamilyState {
    /// The source reports a history record.
    Handled,
    /// The supported build reports no history, so the history is explicitly empty.
    Unavailable,
}

/// Declared support state with the counts the source reports.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticFamilyCoverage {
    /// Whether the source reports a history at all.
    pub state: SemanticFamilyState,
    /// Number of observed events the source declares.
    pub event_count: usize,
    /// Number of disclosed gaps the source declares.
    pub gap_count: usize,
}
