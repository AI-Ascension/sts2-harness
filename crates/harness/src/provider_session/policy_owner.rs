// SPDX-License-Identifier: MIT

//! Durable, explicit adoption history for saved provider-session policies.

use super::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionMetadataStoreError,
    ProviderSessionPolicy, SessionPolicyMigrationProposal, SessionPolicyMigrationState,
    SessionScope,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

mod metadata;
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

#[path = "policy_owner/change.rs"]
mod change;
#[path = "policy_owner/owner_impl.rs"]
mod owner_impl;

#[path = "policy_owner/journal.rs"]
mod journal;
use journal::{persist_candidate, validate_journal};

#[cfg(test)]
#[path = "policy_owner/tests.rs"]
mod tests;
