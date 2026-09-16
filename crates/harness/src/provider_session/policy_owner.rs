// SPDX-License-Identifier: MIT

//! Durable, explicit adoption history for saved provider-session policies.

use super::{
    NativeCapabilities, PolicyOwnerLease, ProviderSessionMetadataStore,
    ProviderSessionMetadataStoreError, ProviderSessionPolicy, SessionPolicyMigrationProposal,
    SessionPolicyMigrationState, SessionScope,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

mod commands;
mod journal;
mod metadata;
#[cfg(test)]
#[path = "policy_owner/tests.rs"]
mod tests;
use journal::validate_journal;
pub use metadata::{
    ProviderSessionPolicyMetadata, ProviderSessionPolicyOwnerMetadata,
    ProviderSessionPolicyProposalMetadata,
};

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
    /// Another live policy owner holds this journal's exclusive lease.
    Busy,
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
    _lease: PolicyOwnerLease,
    journal: Mutex<Journal>,
}

impl ProviderSessionPolicyOwner {
    /// Opens the single authoritative owner for this durable journal.
    ///
    /// The capability descriptor is structurally validated before the journal is loaded. A
    /// competing live owner returns `Busy`; callers must use the already open owner instance.
    pub fn open(
        store: ProviderSessionMetadataStore,
        scope: SessionScope,
        capabilities: NativeCapabilities,
    ) -> Result<Self, ProviderSessionPolicyOwnerError> {
        if !scope.valid() || store.mode() != super::ProviderSessionMetadataMode::EncryptedPersistent
        {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        capabilities
            .validate()
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let lease = store
            .acquire_owner_journal_lease()
            .map_err(|error| match error {
                ProviderSessionMetadataStoreError::Busy => ProviderSessionPolicyOwnerError::Busy,
                _ => ProviderSessionPolicyOwnerError::Store,
            })?;
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
        lease
            .verify()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        validate_journal(&journal, &scope, &capabilities)?;
        Ok(Self {
            store,
            scope,
            capabilities,
            _lease: lease,
            journal: Mutex::new(journal),
        })
    }

    pub(super) fn verify_lease(&self) -> Result<(), ProviderSessionPolicyOwnerError> {
        self._lease
            .verify()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)
    }
}
