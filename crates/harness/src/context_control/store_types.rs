// SPDX-License-Identifier: MIT

use std::fmt::{Display, Formatter};

pub const CURRENT_CONTEXT_CONTROL_SCHEMA_VERSION: i64 = 1;
pub(super) const STORE_SCHEMA: &str = "ascension.context-control.sqlite.v1";
pub(super) const AAD: &[u8] = b"ascension.context-control.sqlite.v1\0";
pub(super) const MAX_JOURNAL_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_EVENTS: usize = 4096;
pub(super) const MAX_EVENT_BYTES: usize = 64 * 1024;

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
