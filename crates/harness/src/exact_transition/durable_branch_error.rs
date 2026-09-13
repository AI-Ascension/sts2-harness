// SPDX-License-Identifier: MIT

use std::fmt;

/// Durable operation errors for branch metadata and retention.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BranchStoreError {
    /// Input was empty, oversized, malformed, or inconsistent.
    InvalidInput,
    /// Requested experiment does not exist.
    UnknownExperiment,
    /// Requested branch does not exist.
    UnknownBranch,
    /// Requested parent branch does not exist in the experiment.
    UnknownParent,
    /// An immutable identity or scope is already allocated.
    Duplicate,
    /// Operation ID was reused with a different payload.
    IdempotencyConflict,
    /// CAS metadata revision was stale.
    StaleRevision,
    /// Lifecycle transition is not allowed.
    InvalidTransition,
    /// Strategy evidence is insufficient for readiness.
    InsufficientAssurance,
    /// Configured branch or artifact bound was reached.
    Capacity,
    /// A requested artifact is unavailable or already tombstoned.
    ArtifactUnavailable,
    /// A durable cursor is malformed or outside its bound.
    InvalidCursor,
    /// SQLite schema is newer than this owner.
    UnsupportedSchema,
    /// Persisted rows violated the owner contract.
    Corrupt,
    /// The SQLite or filesystem boundary failed.
    Persistence(String),
}

impl BranchStoreError {
    pub(crate) fn persistence(error: impl fmt::Display) -> Self {
        Self::Persistence(error.to_string())
    }
}

impl fmt::Display for BranchStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidInput => "durable branch input is invalid",
            Self::UnknownExperiment => "durable branch experiment was not found",
            Self::UnknownBranch => "durable branch was not found",
            Self::UnknownParent => "durable branch parent was not found",
            Self::Duplicate => "durable branch identity or scope already exists",
            Self::IdempotencyConflict => "branch operation payload conflicts with its prior result",
            Self::StaleRevision => "durable branch metadata revision is stale",
            Self::InvalidTransition => "durable branch lifecycle transition is invalid",
            Self::InsufficientAssurance => "strategy evidence is insufficient for readiness",
            Self::Capacity => "durable branch capacity has been reached",
            Self::ArtifactUnavailable => "durable branch artifact is unavailable",
            Self::InvalidCursor => "durable branch cursor is invalid",
            Self::UnsupportedSchema => "durable branch schema is newer than this owner",
            Self::Corrupt => "durable branch storage is corrupt",
            Self::Persistence(_) => "durable branch storage failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for BranchStoreError {}
