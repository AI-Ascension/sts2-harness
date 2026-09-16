// SPDX-License-Identifier: MIT

use super::{BranchStrategy, MAX_BRANCHES, MAX_TRANSITION_LABEL_BYTES, OccurrenceId};
use crate::execution::ExactStateDigest;

#[path = "durable_branch_artifacts.rs"]
mod branch_artifacts;
#[path = "durable_branch_store.rs"]
mod store;
#[path = "durable_branch_store_artifacts.rs"]
mod store_artifacts;
#[path = "durable_branch_store_bulk_reads.rs"]
mod store_bulk_reads;
#[path = "durable_branch_store_create.rs"]
mod store_create;
#[path = "durable_branch_error.rs"]
mod store_error;
#[path = "store_migration.rs"]
mod store_migration;
#[path = "durable_branch_store_mutations.rs"]
mod store_mutations;
#[path = "durable_branch_store_prune_plan.rs"]
mod store_prune_plan;
#[path = "durable_branch_store_reachability.rs"]
mod store_reachability;
#[path = "durable_branch_store_read_helpers.rs"]
mod store_read_helpers;
#[path = "durable_branch_store_reads.rs"]
mod store_reads;
#[path = "durable_branch_store_reconciliation.rs"]
mod store_reconciliation;
#[path = "durable_branch_store_retention.rs"]
mod store_retention;
#[path = "durable_branch_validation.rs"]
mod validation;

pub use branch_artifacts::{
    BranchArtifactAvailability, BranchArtifactReference, BranchArtifactResolution,
    BranchArtifactResolver, BranchArtifactRole, BranchArtifactState, BranchArtifactUnavailable,
    ExactArtifactStoreResolver,
};
pub use store::SqliteBranchStore;
pub use store_error::BranchStoreError;
pub use store_reads::{BranchEventPage, BranchPage};
pub use store_retention::{BranchPrunePlan, BranchPruneRequest, BranchRetentionPolicy};
/// Versioned durable branch contract persisted by [`SqliteBranchStore`].
pub const DURABLE_BRANCH_SCHEMA_VERSION: &str = "ascension.durable-branch/v1";
/// Current SQLite migration revision for the durable branch contract.
pub const DURABLE_BRANCH_SCHEMA_REVISION: i64 = 2;
/// Maximum branch name length.
pub const MAX_BRANCH_NAME_BYTES: usize = 128;
/// Maximum notes length.
pub const MAX_BRANCH_NOTES_BYTES: usize = 2_048;
/// Maximum artifact references attached to one branch.
pub const MAX_BRANCH_ARTIFACTS: usize = 32;
/// Maximum branch event page size.
pub const MAX_BRANCH_EVENT_PAGE: u64 = 128;
/// Maximum branch list page size.
pub const MAX_BRANCH_PAGE: u64 = 128;

/// Durable lifecycle state for a branch attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableBranchStatus {
    /// Metadata and fork intent have been committed, but no strategy has started.
    Pending,
    /// Exact native restoration is preparing.
    Restoring,
    /// Public prefix replay is preparing.
    Replaying,
    /// Strategy and context preparation have completed with sufficient evidence.
    Ready,
    /// A continuation has been selected for its independently scoped run.
    Running,
    /// Continuation is paused while retaining its resources and history.
    Held,
    /// Continuation reached a terminal outcome.
    Completed,
    /// Strategy or continuation failed without claiming an effect.
    Failed,
    /// An external effect is uncertain and requires same-operation reconciliation.
    Unknown,
    /// Retained metadata is excluded from ordinary selection.
    Archived,
}

impl DurableBranchStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Restoring => "restoring",
            Self::Replaying => "replaying",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Held => "held",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::Archived => "archived",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, BranchStoreError> {
        match value {
            "pending" => Ok(Self::Pending),
            "restoring" => Ok(Self::Restoring),
            "replaying" => Ok(Self::Replaying),
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "held" => Ok(Self::Held),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "unknown" => Ok(Self::Unknown),
            "archived" => Ok(Self::Archived),
            _ => Err(BranchStoreError::Corrupt),
        }
    }
}

/// Evidence label attached to the selected continuation strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchAssurance {
    /// No strategy evidence has been verified.
    Unverified,
    /// A native exact-restore receipt has been verified by the strategy owner.
    ExactRestoreReceipt,
    /// A public replay boundary has been verified; this remains weaker than exact restore.
    PrefixReplayBoundary,
}

impl BranchAssurance {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::ExactRestoreReceipt => "exact_restore_receipt",
            Self::PrefixReplayBoundary => "prefix_replay_boundary",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, BranchStoreError> {
        match value {
            "unverified" => Ok(Self::Unverified),
            "exact_restore_receipt" => Ok(Self::ExactRestoreReceipt),
            "prefix_replay_boundary" => Ok(Self::PrefixReplayBoundary),
            _ => Err(BranchStoreError::Corrupt),
        }
    }
}

/// The occurrence at which a branch was forked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchFork {
    /// Stable occurrence identity; equal state digests do not merge this identity.
    pub occurrence_id: OccurrenceId,
    /// Existing parent occurrence, when the source graph records one.
    pub parent_occurrence_id: Option<OccurrenceId>,
    /// Exact state digest observed at the fork boundary.
    pub state_digest: ExactStateDigest,
}

/// Input to an atomic branch creation operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableBranchDraft {
    /// Experiment namespace.
    pub experiment_id: String,
    /// Root branch identity for this experiment.
    pub root_branch_id: String,
    /// New immutable branch identity.
    pub branch_id: String,
    /// Existing parent branch, absent only for the experiment root.
    pub parent_branch_id: Option<String>,
    /// Source occurrence and exact state identity.
    pub fork: BranchFork,
    /// Strategy descriptor; assurance is recorded separately.
    pub strategy: BranchStrategy,
    /// Opaque checkpoint/source handle, if supplied.
    pub source_handle: Option<String>,
    /// Public trajectory prefix, if supplied.
    pub trajectory_prefix: Option<String>,
    /// Effective seed or seed label, if supplied.
    pub effective_seed: Option<String>,
    /// Effective setup/configuration digest or label, if supplied.
    pub setup_digest: Option<String>,
    /// Boundary at which the fork was recorded.
    pub boundary: String,
    /// Initial evidence label. New branches always start pending.
    pub assurance: BranchAssurance,
    /// Independent child run identity.
    pub run_id: String,
    /// Optional episode association.
    pub episode_id: Option<String>,
    /// Optional trajectory association.
    pub trajectory_id: Option<String>,
    /// Optional context association.
    pub context_id: Option<String>,
    /// Policy revision used for this branch.
    pub policy_revision: String,
    /// Configuration revision used for this branch.
    pub config_revision: String,
    /// Operator-facing metadata name.
    pub name: String,
    /// Optional operator note, kept outside gameplay identity.
    pub notes: Option<String>,
    /// Reachable artifact references captured with the intent.
    pub artifacts: Vec<BranchArtifactReference>,
}

/// Durable branch record returned by CRUD and read operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableBranch {
    /// Experiment namespace.
    pub experiment_id: String,
    /// Root branch identity.
    pub root_branch_id: String,
    /// Immutable branch identity.
    pub branch_id: String,
    /// Immutable parent edge.
    pub parent_branch_id: Option<String>,
    /// Immutable fork source occurrence.
    pub fork: BranchFork,
    /// Strategy descriptor.
    pub strategy: BranchStrategy,
    /// Opaque source handle, if present.
    pub source_handle: Option<String>,
    /// Public trajectory prefix, if present.
    pub trajectory_prefix: Option<String>,
    /// Effective seed or seed label, if present.
    pub effective_seed: Option<String>,
    /// Effective setup/configuration digest or label, if present.
    pub setup_digest: Option<String>,
    /// Fork boundary label.
    pub boundary: String,
    /// Verified strategy evidence.
    pub assurance: BranchAssurance,
    /// Independent child run identity.
    pub run_id: String,
    /// Optional episode association.
    pub episode_id: Option<String>,
    /// Optional trajectory association.
    pub trajectory_id: Option<String>,
    /// Optional context association.
    pub context_id: Option<String>,
    /// Policy revision.
    pub policy_revision: String,
    /// Configuration revision.
    pub config_revision: String,
    /// Operator-facing name.
    pub name: String,
    /// Operator note.
    pub notes: Option<String>,
    /// Current lifecycle state.
    pub status: DurableBranchStatus,
    /// CAS revision for metadata and lifecycle mutations.
    pub metadata_revision: u64,
    /// Wall timestamp in Unix milliseconds.
    pub created_at: i64,
    /// Last mutation timestamp in Unix milliseconds.
    pub updated_at: i64,
    /// Reachable artifact references that have not been tombstoned.
    pub artifacts: Vec<BranchArtifactReference>,
}

/// One append-only durable branch event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchEvent {
    /// Monotonic database cursor.
    pub sequence: u64,
    /// Experiment namespace.
    pub experiment_id: String,
    /// Branch affected by the event.
    pub branch_id: String,
    /// Operation that produced the event.
    pub operation_id: String,
    /// Stable event kind.
    pub kind: String,
    /// Branch status after the event.
    pub status: DurableBranchStatus,
    /// Metadata revision after the event.
    pub metadata_revision: u64,
    /// Wall timestamp in Unix milliseconds.
    pub occurred_at: i64,
}
