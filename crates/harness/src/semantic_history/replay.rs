// SPDX-License-Identifier: MIT

//! Idempotent append across a restart, a replayed batch and a branch fork.
//!
//! A batch may be re-delivered after a restart, and a branch fork replays its ancestor's history
//! before appending what is new. Both are handled by identity rather than by trusting the caller: an
//! append whose payload is already retained is a no-op, and one that reuses an append identity with
//! a different payload is refused instead of overwriting.

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::record::SemanticEventBatch;
use super::scope::{SemanticCatalogBinding, SemanticEventScope};
use serde::{Deserialize, Serialize};

/// One append request: the caller's operation identity, the binding, and the batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryAppend {
    /// Caller-owned operation identity, unique to this append.
    pub operation_id: String,
    /// Catalog binding the batch was observed against.
    pub binding: SemanticCatalogBinding,
    /// The batch to retain.
    pub batch: SemanticEventBatch,
}

/// What one append did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticAppendOutcome {
    /// New records were retained.
    Appended {
        /// How many records this append added.
        added: usize,
        /// How many records the history now holds.
        total: usize,
    },
    /// The identical payload was already retained, so nothing changed.
    AlreadyPresent {
        /// How many records the history holds.
        total: usize,
    },
}

impl SemanticAppendOutcome {
    /// Returns how many records the history holds after this append.
    #[must_use]
    pub const fn total(&self) -> usize {
        match self {
            Self::Appended { total, .. } | Self::AlreadyPresent { total } => *total,
        }
    }

    /// Returns whether this append changed the retained history.
    #[must_use]
    pub const fn changed(&self) -> bool {
        matches!(self, Self::Appended { .. })
    }
}

/// One fork request: a child branch that inherits its ancestor's retained history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryFork {
    /// Caller-owned operation identity, unique to this fork.
    pub operation_id: String,
    /// The retained branch this fork descends from.
    pub parent_branch_id: String,
    /// The run, branch, episode and epoch the child branch will own.
    pub child: SemanticEventScope,
}

/// What one fork did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticForkOutcome {
    /// How many ancestor records the child inherited by lineage.
    pub inherited: usize,
    /// How many records the child branch now holds.
    pub total: usize,
    /// Whether this fork identity was already recorded, so nothing changed.
    pub already_present: bool,
}

impl SemanticForkOutcome {
    /// Returns whether this fork changed the retained history.
    #[must_use]
    pub const fn changed(&self) -> bool {
        !self.already_present
    }
}

/// Validates a fork request's own shape before any store work.
///
/// A fork that names its own branch as its ancestor, or that names no ancestor, is refused rather
/// than resolved to something plausible.
pub(super) fn validate_fork(fork: &SemanticHistoryFork) -> SemanticHistoryResult<()> {
    if fork.operation_id.is_empty() {
        return Err(SemanticHistoryError::new(Refusal::Identity));
    }
    if fork.parent_branch_id.is_empty() {
        return Err(SemanticHistoryError::new(Refusal::UnknownParentBranch));
    }
    if fork.parent_branch_id == fork.child.branch_id {
        return Err(SemanticHistoryError::new(Refusal::SelfParentBranch));
    }
    if fork.child.run_id.is_empty() || fork.child.branch_id.is_empty() {
        return Err(SemanticHistoryError::new(Refusal::Identity));
    }
    Ok(())
}
