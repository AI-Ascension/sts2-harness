// SPDX-License-Identifier: MIT

//! Owner-supplied scope and branch lineage for one history store.

use super::{Error, validate_history_identity};
use serde::{Deserialize, Serialize};

/// The owner scope one history store serves.
///
/// The scope is supplied by the owner and is never decoded from game text or model output, so a
/// cross-run or cross-profile query cannot be reached by asking for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryBinding {
    /// Project the history belongs to.
    pub project_id: String,
    /// Run the history belongs to.
    pub run_id: String,
    /// Agent the history belongs to.
    pub agent_id: String,
    /// Game profile the history was captured under.
    pub game_profile: String,
    /// Content manifest the history binds to.
    pub content_manifest_id: String,
    /// Locale the history binds to.
    pub locale: String,
    /// Authority epoch this store serves.
    pub authority_epoch: u64,
}

impl SemanticHistoryBinding {
    /// Validates every field.
    pub fn validate(&self) -> Result<(), Error> {
        validate_history_identity(&self.project_id, "binding.project_id")?;
        validate_history_identity(&self.run_id, "binding.run_id")?;
        validate_history_identity(&self.agent_id, "binding.agent_id")?;
        validate_history_identity(&self.game_profile, "binding.game_profile")?;
        validate_history_identity(&self.content_manifest_id, "binding.content_manifest_id")?;
        validate_history_identity(&self.locale, "binding.locale")?;
        Ok(())
    }

    /// Returns whether two bindings describe the same owner scope.
    ///
    /// The epoch is deliberately excluded: a store may be advanced to a new epoch without changing
    /// which owner it serves, and `same_owner` is what a cross-run query is refused on.
    #[must_use]
    pub fn same_owner(&self, other: &Self) -> bool {
        self.project_id == other.project_id
            && self.run_id == other.run_id
            && self.agent_id == other.agent_id
            && self.game_profile == other.game_profile
            && self.content_manifest_id == other.content_manifest_id
            && self.locale == other.locale
    }
}

/// One branch edge in the lineage this store follows.
///
/// A branch is an independent writable history scope. The lineage records which branch forked from
/// which, so a query can follow ancestry without re-deriving it from sequence numbers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryLineage {
    /// The branch this edge describes.
    pub branch_id: String,
    /// The branch it forked from; absent only for the run root.
    pub parent_branch_id: Option<String>,
    /// The host sequence at which the fork happened.
    pub fork_sequence: u64,
    /// The authority epoch the child branch started in.
    pub authority_epoch: u64,
}

impl SemanticHistoryLineage {
    /// Validates the edge.
    pub fn validate(&self) -> Result<(), Error> {
        validate_history_identity(&self.branch_id, "lineage.branch_id")?;
        if let Some(parent) = &self.parent_branch_id {
            validate_history_identity(parent, "lineage.parent_branch_id")?;
            if parent == &self.branch_id {
                // A branch that forked from itself would make ancestry a cycle.
                return Err(Error::Lineage);
            }
        }
        Ok(())
    }
}
