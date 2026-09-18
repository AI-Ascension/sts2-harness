// SPDX-License-Identifier: MIT

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use super::types::ContextSourceDocument;
use crate::management::{ContextControlCommand, ContextControlReceipt, ContextOwnerBinding};

pub const CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION: i64 = 1;
pub(super) const STORE_SCHEMA: &str = "ascension.context-control.sqlite.v1";
pub(super) const AAD: &[u8] = b"ascension.context-control.sqlite.v1\0";
pub(super) const MAX_CONTEXT_SOURCE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_JOURNAL_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_EVENTS: usize = 4096;
pub(super) const MAX_EVENT_BYTES: usize = 64 * 1024;
pub(super) const MAX_OWNER_RECEIPT_BYTES: usize = 64 * 1024;
/// Upper bound on the persisted logical-invocation lifetime state for one run.
pub(super) const MAX_LIFETIME_STATE_BYTES: usize = 1024 * 1024;

/// Encrypted, owner-scoped evidence for one already applied context-control command.
///
/// The durable store persists this record atomically with the resulting authority journal.
/// Reading it does not acquire the runtime or owner fence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableContextOwnerControlReceipt {
    pub owner_id: String,
    pub actor_subject: String,
    pub binding: ContextOwnerBinding,
    pub command: ContextControlCommand,
    pub receipt: ContextControlReceipt,
}

/// Owner-published immutable bytes retained encrypted by the control store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableContextSourceSnapshot {
    pub source_id: String,
    pub version: u64,
    pub digest: String,
    pub document: ContextSourceDocument,
}

/// Typed active-source pointer committed with the authority journal and receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableActiveContextSource {
    pub source_id: String,
    pub version: u64,
    pub digest: String,
    pub active_revision_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreMode {
    Disabled,
    Enabled,
}

impl StoreMode {
    pub(super) fn as_i64(self) -> i64 {
        match self {
            Self::Disabled => 0,
            Self::Enabled => 1,
        }
    }

    pub(super) fn from_i64(value: i64) -> Result<Self, DurableControlStoreError> {
        match value {
            0 => Ok(Self::Disabled),
            1 => Ok(Self::Enabled),
            _ => Err(DurableControlStoreError::Corrupt),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableStoreFailpoint {
    BeforeJournalWrite,
    BeforeCommit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreSnapshot {
    pub run_id: String,
    pub mode: StoreMode,
    pub journal_bytes: usize,
    pub journal_digest: String,
    pub phase1_snapshot_count: usize,
    pub phase1_snapshot_digests: Vec<String>,
    pub outbox_event_count: usize,
    pub outbox_event_digests: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum DurableControlStoreError {
    InvalidPath,
    ParentMissing,
    InvalidKey,
    Sqlite,
    Encode,
    Decode,
    AuthenticationFailed,
    Corrupt,
    Incompatible,
    Missing,
    ScopeMismatch,
    Fenced,
    TooLarge,
    Failpoint,
    SnapshotConflict,
    InvalidSnapshotId,
    SourceConflict,
    InvalidSourceId,
    OwnerReceiptConflict,
}

impl Display for DurableControlStoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPath => "control store path is invalid",
            Self::ParentMissing => "control store parent is missing",
            Self::InvalidKey => "control store key is invalid",
            Self::Sqlite => "control store database operation failed",
            Self::Encode => "control journal could not be encoded",
            Self::Decode => "control journal could not be decoded",
            Self::AuthenticationFailed => "control journal authentication failed",
            Self::Corrupt => "control store integrity check failed",
            Self::Incompatible => "control store schema is incompatible",
            Self::Missing => "control journal is unavailable",
            Self::ScopeMismatch => "control journal scope does not match this store",
            Self::Fenced => "control store owner fence rejected the operation",
            Self::TooLarge => "control store object exceeds its bound",
            Self::Failpoint => "control store failpoint rejected the transaction",
            Self::SnapshotConflict => "phase1 snapshot already has different bytes",
            Self::InvalidSnapshotId => "phase1 snapshot identity is invalid",
            Self::SourceConflict => "context source identity already has different bytes",
            Self::InvalidSourceId => "context source identity is invalid",
            Self::OwnerReceiptConflict => {
                "context control idempotency key is already bound to different receipt evidence"
            }
        })
    }
}

impl std::error::Error for DurableControlStoreError {}

#[derive(Debug, Eq, PartialEq)]
pub enum LegacyOpenError {
    NotFound,
    Sqlite,
    ManagementActive,
    Incompatible,
}

impl Display for LegacyOpenError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "legacy store does not exist",
            Self::Sqlite => "legacy store could not be inspected",
            Self::ManagementActive => "legacy binary refused management-active store",
            Self::Incompatible => "legacy store marker is incompatible",
        })
    }
}

impl std::error::Error for LegacyOpenError {}
