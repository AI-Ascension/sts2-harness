// SPDX-License-Identifier: MIT

use super::records::SavedPolicyRef;
use crate::context_memory::{MemoryError, MEMORY_POLICY_MIGRATION_MAX_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_POLICY_BYTES: usize = if MEMORY_POLICY_MIGRATION_MAX_BYTES < 65_536 {
    MEMORY_POLICY_MIGRATION_MAX_BYTES
} else { 65_536 };
pub const MAX_POLICY_VERSIONS: usize = 128;
pub const MAX_POLICY_REVIEWS: usize = 64;
pub const MAX_POLICY_RECEIPTS: usize = 512;
pub const MAX_POLICY_GRANTS: usize = 64;
pub const MAX_POLICY_JOURNAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_POLICY_DATABASE_BYTES: u64 = 32 * 1024 * 1024;

/// Ephemeral credentials; intentionally neither Debug nor serializable.
pub struct PolicyAccess<'a> {
    pub bearer: Option<&'a str>,
    pub grant_id: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyPermission { ReadMetadata, ReadContent, Write, Approve, Adopt, Select }

/// Trusted configuration, never a command field.
#[derive(Clone, Debug)]
pub struct PolicyGrant {
    pub grant_id: String,
    pub subject: String,
    pub scope: crate::context_memory::MemoryScope,
    pub permissions: BTreeSet<PolicyPermission>,
    pub epoch: u64,
    pub expires_at: u64,
    pub revoked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyOwnerError {
    Unauthenticated, PermissionDenied, GrantRevoked, ScopeMismatch, SchemaInvalid,
    StaleReview, OwnerFenced, Conflict, Capacity, StoreIncompatible, Corrupt,
    Unavailable, Missing, LostReply, PersistenceFailure, Memory(MemoryError),
}

impl std::fmt::Display for PolicyOwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PolicyOwnerError {}
impl From<MemoryError> for PolicyOwnerError {
    fn from(value: MemoryError) -> Self { Self::Memory(value) }
}

/// Closed internal commands, not an advertised transport protocol. Raw bodies have no Debug.
#[derive(Clone, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PolicyCommand {
    Import { key: String, raw: Vec<u8> },
    ProposeMigration {
        key: String, review_id: String, source: SavedPolicyRef, target_raw: Vec<u8>,
        expected_active_version: Option<u64>,
    },
    ProposeRevalidation {
        key: String, review_id: String, source: SavedPolicyRef, target_raw: Vec<u8>,
        expected_active_version: u64,
    },
    Approve { key: String, review_id: String, review_sha256: String },
    Adopt { key: String, review_id: String, review_sha256: String },
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
    })
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl PolicyCommand {
    pub(super) fn key(&self) -> &str {
        match self {
            Self::Import { key, .. } | Self::ProposeMigration { key, .. }
            | Self::ProposeRevalidation { key, .. } | Self::Approve { key, .. }
            | Self::Adopt { key, .. } => key,
        }
    }
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::Import { .. } => "import",
            Self::ProposeMigration { .. } => "propose_migration",
            Self::ProposeRevalidation { .. } => "propose_revalidation",
            Self::Approve { .. } => "approve",
            Self::Adopt { .. } => "adopt",
        }
    }
    pub(super) fn permission(&self) -> PolicyPermission {
        match self {
            Self::Approve { .. } => PolicyPermission::Approve,
            Self::Adopt { .. } => PolicyPermission::Adopt,
            _ => PolicyPermission::Write,
        }
    }
    pub(super) fn validate(&self) -> Result<(), PolicyOwnerError> {
        if !valid_id(self.key()) { return Err(PolicyOwnerError::SchemaInvalid); }
        match self {
            Self::Import { raw, .. } => check_raw(raw),
            Self::ProposeMigration { review_id, source, target_raw, .. }
            | Self::ProposeRevalidation { review_id, source, target_raw, .. } => {
                if !valid_id(review_id) || !source.valid() {
                    return Err(PolicyOwnerError::SchemaInvalid);
                }
                check_raw(target_raw)
            }
            Self::Approve { review_id, review_sha256, .. }
            | Self::Adopt { review_id, review_sha256, .. } => {
                if valid_id(review_id) && valid_digest(review_sha256) { Ok(()) }
                else { Err(PolicyOwnerError::SchemaInvalid) }
            }
        }
    }
    pub(super) fn fingerprint(&self) -> Result<String, PolicyOwnerError> {
        let bytes = serde_json::to_vec(self).map_err(|_| PolicyOwnerError::SchemaInvalid)?;
        Ok(crate::context_memory::sha256_hex(bytes))
    }
}

pub(super) fn check_raw(raw: &[u8]) -> Result<(), PolicyOwnerError> {
    if raw.is_empty() || raw.len() > MAX_POLICY_BYTES { Err(PolicyOwnerError::Capacity) }
    else { Ok(()) }
}
