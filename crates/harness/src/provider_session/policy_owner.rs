// SPDX-License-Identifier: MIT

//! Durable, explicit adoption history for saved provider-session policies.

use super::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionMetadataStoreError,
    ProviderSessionPolicy, SessionPolicyMigrationProposal, SessionPolicyMigrationState,
    SessionScope,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

const SCHEMA: &str = "ascension.provider-session.policy-owner.v1";
const MAX_RECORDS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyRecord {
    pub sha256: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    id: String,
    digest: String,
    source_sha256: String,
    target_sha256: String,
    migration: SessionPolicyMigrationProposal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: String,
    revision: u64,
    policies: Vec<ProviderSessionPolicyRecord>,
    proposals: Vec<Proposal>,
    active_sha256: Option<String>,
}

#[derive(Debug)]
pub enum ProviderSessionPolicyOwnerError {
    Invalid,
    Missing,
    Conflict,
    NotAdopted,
    Store,
}

impl std::fmt::Display for ProviderSessionPolicyOwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProviderSessionPolicyOwnerError {}

/// Encrypted saved-policy history scoped to one stable provider-session owner.
pub struct ProviderSessionPolicyOwner {
    store: ProviderSessionMetadataStore,
    scope: SessionScope,
    capabilities: NativeCapabilities,
    journal: Mutex<Journal>,
}

impl ProviderSessionPolicyOwner {
    pub fn open(
        store: ProviderSessionMetadataStore,
        scope: SessionScope,
        capabilities: NativeCapabilities,
    ) -> Result<Self, ProviderSessionPolicyOwnerError> {
        if !scope.valid() || store.mode() != super::ProviderSessionMetadataMode::EncryptedPersistent
        {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        let journal = match store.load_owner_journal() {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| ProviderSessionPolicyOwnerError::Store)?,
            Err(ProviderSessionMetadataStoreError::NotFound) => Journal {
                schema: SCHEMA.to_owned(),
                revision: 1,
                policies: Vec::new(),
                proposals: Vec::new(),
                active_sha256: None,
            },
            Err(_) => return Err(ProviderSessionPolicyOwnerError::Store),
        };
        if journal.schema != SCHEMA
            || journal.revision == 0
            || journal.policies.len() > MAX_RECORDS
            || journal.proposals.len() > MAX_RECORDS
        {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        Ok(Self {
            store,
            scope,
            capabilities,
            journal: Mutex::new(journal),
        })
    }

    pub fn import(&self, bytes: Vec<u8>) -> Result<String, ProviderSessionPolicyOwnerError> {
        let policy: ProviderSessionPolicy =
            serde_json::from_slice(&bytes).map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        policy
            .validate_schema()
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        if policy.scope != self.scope || bytes.len() > super::MAX_HISTORY_BYTES {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        let sha256 = crate::sha256_hex(&bytes);
        self.change(|journal| {
            if journal.policies.iter().any(|item| item.sha256 == sha256) {
                return Ok(sha256.clone());
            }
            if journal.policies.len() >= MAX_RECORDS {
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            journal.policies.push(ProviderSessionPolicyRecord {
                sha256: sha256.clone(),
                bytes,
            });
            Ok(sha256.clone())
        })
    }

    pub fn propose(
        &self,
        id: &str,
        source_sha256: &str,
        target: Vec<u8>,
        expected_revision: u64,
    ) -> Result<String, ProviderSessionPolicyOwnerError> {
        self.change(|journal| {
            if journal.revision != expected_revision
                || journal.proposals.iter().any(|item| item.id == id)
            {
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            let source_bytes = journal
                .policies
                .iter()
                .find(|item| item.sha256 == source_sha256)
                .map(|item| item.bytes.clone())
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            let target_sha256 = self.import_into(journal, target)?;
            let migration = SessionPolicyMigrationProposal::new_from_bytes(
                &source_bytes,
                &self.capabilities,
                id,
            )
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
            let digest = crate::sha256_hex(
                serde_json::to_vec(&(id, source_sha256, &target_sha256, &migration))
                    .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?,
            );
            journal.proposals.push(Proposal {
                id: id.to_owned(),
                digest: digest.clone(),
                source_sha256: source_sha256.to_owned(),
                target_sha256,
                migration,
            });
            Ok(digest)
        })
    }

    pub fn approve(
        &self,
        id: &str,
        digest: &str,
        approval_ref: &str,
    ) -> Result<(), ProviderSessionPolicyOwnerError> {
        self.change(|journal| {
            let proposal = journal
                .proposals
                .iter_mut()
                .find(|item| item.id == id && item.digest == digest)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            proposal
                .migration
                .approve(approval_ref)
                .map_err(|_| ProviderSessionPolicyOwnerError::Conflict)
        })
    }

    pub fn adopt(
        &self,
        id: &str,
        digest: &str,
        approval_ref: &str,
        expected_revision: u64,
    ) -> Result<(), ProviderSessionPolicyOwnerError> {
        self.change(|journal| {
            if journal.revision != expected_revision {
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            let proposal = journal
                .proposals
                .iter_mut()
                .find(|item| item.id == id && item.digest == digest)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            let target = journal
                .policies
                .iter()
                .find(|item| item.sha256 == proposal.target_sha256)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            let policy: ProviderSessionPolicy = serde_json::from_slice(&target.bytes)
                .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
            proposal
                .migration
                .adopt(&policy, &self.capabilities, approval_ref)
                .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
            if proposal.migration.state != SessionPolicyMigrationState::Adopted {
                return Err(ProviderSessionPolicyOwnerError::NotAdopted);
            }
            journal.active_sha256 = Some(proposal.target_sha256.clone());
            Ok(())
        })
    }

    pub fn active(
        &self,
    ) -> Result<(ProviderSessionPolicy, String, u64), ProviderSessionPolicyOwnerError> {
        let journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        let active = journal
            .active_sha256
            .as_ref()
            .ok_or(ProviderSessionPolicyOwnerError::NotAdopted)?;
        let record = journal
            .policies
            .iter()
            .find(|item| &item.sha256 == active)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let policy = serde_json::from_slice(&record.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        Ok((policy, active.clone(), journal.revision))
    }

    fn import_into(
        &self,
        journal: &mut Journal,
        bytes: Vec<u8>,
    ) -> Result<String, ProviderSessionPolicyOwnerError> {
        let policy: ProviderSessionPolicy =
            serde_json::from_slice(&bytes).map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        policy
            .validate_schema()
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        if policy.scope != self.scope || bytes.len() > super::MAX_HISTORY_BYTES {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        let sha256 = crate::sha256_hex(&bytes);
        if !journal.policies.iter().any(|item| item.sha256 == sha256) {
            if journal.policies.len() >= MAX_RECORDS {
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            journal.policies.push(ProviderSessionPolicyRecord {
                sha256: sha256.clone(),
                bytes,
            });
        }
        Ok(sha256)
    }
    fn change<T>(
        &self,
        f: impl FnOnce(&mut Journal) -> Result<T, ProviderSessionPolicyOwnerError>,
    ) -> Result<T, ProviderSessionPolicyOwnerError> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        let mut candidate = journal.clone();
        let result = f(&mut candidate)?;
        candidate.revision = candidate
            .revision
            .checked_add(1)
            .ok_or(ProviderSessionPolicyOwnerError::Conflict)?;
        let bytes =
            serde_json::to_vec(&candidate).map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        self.store
            .save_owner_journal(&bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        *journal = candidate;
        Ok(result)
    }
}
