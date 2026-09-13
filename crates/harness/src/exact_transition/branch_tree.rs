// SPDX-License-Identifier: MIT

//! Immutable, scoped branch metadata for checkpoint experiments.
//!
//! This is the owner-side record model. A storage adapter must persist a completed operation
//! together with its branch record before exposing it; the model itself deliberately has no
//! restore or game-process authority.

use std::collections::BTreeMap;

use super::{MAX_BRANCHES, MAX_TRANSITION_LABEL_BYTES, OccurrenceId};

/// State of a branch before a runtime strategy takes ownership of a child run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchStatus {
    /// The durable intent exists but no strategy has prepared the child.
    Pending,
    /// A selected strategy is preparing independently scoped resources.
    Preparing,
    /// The child may be selected for a continuation attempt.
    Ready,
    /// A strategy could not establish the requested continuation.
    Failed,
    /// The branch is retained but excluded from ordinary selection.
    Archived,
}

/// The evidence required by the selected continuation strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchStrategy {
    /// Requires a verified exact checkpoint restore receipt.
    ExactRestore,
    /// Requires a separately verified public replay boundary.
    PrefixReplay,
}

/// One immutable branch edge and its independently allocated write scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchRecord {
    /// Stable branch identity, distinct from a gameplay-state digest.
    pub branch_id: String,
    /// Parent branch; absent only for the experiment root.
    pub parent_branch_id: Option<String>,
    /// Occurrence at which this child was forked.
    pub fork_occurrence: OccurrenceId,
    /// Separately allocated child run identity.
    pub run_id: String,
    /// Unique writable artifact scope for this branch.
    pub write_scope: String,
    /// Chosen continuation strategy; assurance is not implied by this enum.
    pub strategy: BranchStrategy,
    /// Current durable lifecycle state.
    pub status: BranchStatus,
}

/// Rejection reasons for branch-tree mutations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchTreeError {
    /// An identifier, scope, or operation key is malformed.
    InvalidLabel,
    /// The requested parent does not exist.
    UnknownParent,
    /// A branch or write scope is already allocated.
    Duplicate,
    /// Retrying an operation changed its immutable request payload.
    IdempotencyConflict,
    /// The branch count reached its configured bound.
    Capacity,
    /// The requested lifecycle transition is not permitted.
    InvalidTransition,
}

/// Bounded append-only branch edges with idempotent create operations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BranchTree {
    branches: BTreeMap<String, BranchRecord>,
    operations: BTreeMap<String, BranchRecord>,
}

impl BranchTree {
    /// Creates an empty tree. Insert its root explicitly so its provenance is recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the branch count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.branches.len()
    }

    /// Returns whether the tree is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty()
    }

    /// Looks up one branch by its stable identity.
    #[must_use]
    pub fn get(&self, branch_id: &str) -> Option<&BranchRecord> {
        self.branches.get(branch_id)
    }

    /// Returns all branch records in stable identity order.
    #[must_use]
    pub fn branches(&self) -> Vec<&BranchRecord> {
        self.branches.values().collect()
    }

    /// Records a root or child branch exactly once for an operation key.
    ///
    /// A retry with byte-identical logical fields returns the original record. A changed retry is
    /// rejected rather than silently rebinding a branch or writable destination.
    pub fn create(
        &mut self,
        operation_id: &str,
        record: BranchRecord,
    ) -> Result<&BranchRecord, BranchTreeError> {
        validate_operation(operation_id)?;
        validate_record(&record)?;
        if let Some(existing) = self.operations.get(operation_id) {
            return if existing == &record {
                self.branches
                    .get(&record.branch_id)
                    .ok_or(BranchTreeError::Duplicate)
            } else {
                Err(BranchTreeError::IdempotencyConflict)
            };
        }
        if self.branches.len() >= MAX_BRANCHES {
            return Err(BranchTreeError::Capacity);
        }
        if let Some(parent) = &record.parent_branch_id
            && !self.branches.contains_key(parent)
        {
            return Err(BranchTreeError::UnknownParent);
        }
        if self.branches.contains_key(&record.branch_id)
            || self
                .branches
                .values()
                .any(|branch| branch.write_scope == record.write_scope)
        {
            return Err(BranchTreeError::Duplicate);
        }
        let branch_id = record.branch_id.clone();
        self.branches.insert(branch_id.clone(), record.clone());
        self.operations.insert(operation_id.to_owned(), record);
        self.branches
            .get(&branch_id)
            .ok_or(BranchTreeError::Duplicate)
    }

    /// Advances a branch through the non-effectful metadata lifecycle.
    pub fn transition(
        &mut self,
        branch_id: &str,
        status: BranchStatus,
    ) -> Result<&BranchRecord, BranchTreeError> {
        let record = self
            .branches
            .get_mut(branch_id)
            .ok_or(BranchTreeError::UnknownParent)?;
        if !allowed_transition(record.status, status) {
            return Err(BranchTreeError::InvalidTransition);
        }
        record.status = status;
        Ok(record)
    }
}

fn validate_record(record: &BranchRecord) -> Result<(), BranchTreeError> {
    for value in [&record.branch_id, &record.run_id, &record.write_scope] {
        validate_label(value)?;
    }
    if let Some(parent) = &record.parent_branch_id {
        validate_label(parent)?;
        if parent == &record.branch_id {
            return Err(BranchTreeError::InvalidLabel);
        }
    }
    Ok(())
}

fn validate_operation(value: &str) -> Result<(), BranchTreeError> {
    validate_label(value)
}

fn validate_label(value: &str) -> Result<(), BranchTreeError> {
    if value.is_empty()
        || value.len() > MAX_TRANSITION_LABEL_BYTES
        || value.contains('\0')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
    {
        return Err(BranchTreeError::InvalidLabel);
    }
    Ok(())
}

fn allowed_transition(from: BranchStatus, to: BranchStatus) -> bool {
    matches!(
        (from, to),
        (
            BranchStatus::Pending,
            BranchStatus::Preparing | BranchStatus::Archived
        ) | (
            BranchStatus::Preparing,
            BranchStatus::Ready | BranchStatus::Failed | BranchStatus::Archived
        ) | (BranchStatus::Ready, BranchStatus::Archived)
            | (BranchStatus::Failed, BranchStatus::Archived)
            | (BranchStatus::Archived, BranchStatus::Pending)
    )
}
