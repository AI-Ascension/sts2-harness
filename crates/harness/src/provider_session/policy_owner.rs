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
        validate_journal(&journal, &scope, &capabilities)?;
        Ok(Self {
            store,
            scope,
            capabilities,
            journal: Mutex::new(journal),
        })
    }

    pub fn import(&self, bytes: Vec<u8>) -> Result<String, ProviderSessionPolicyOwnerError> {
        let revision = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?
            .revision;
        self.import_at_revision(bytes, revision)
    }

    pub fn import_at_revision(
        &self,
        bytes: Vec<u8>,
        expected_revision: u64,
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
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        if journal.policies.iter().any(|item| item.sha256 == sha256) {
            return Ok(sha256);
        }
        if journal.revision != expected_revision {
            return Err(ProviderSessionPolicyOwnerError::Conflict);
        }
        let mut candidate = journal.clone();
        if candidate.policies.len() >= MAX_RECORDS {
            return Err(ProviderSessionPolicyOwnerError::Conflict);
        }
        candidate.policies.push(ProviderSessionPolicyRecord {
            sha256: sha256.clone(),
            bytes,
        });
        persist_candidate(&self.store, &mut journal, candidate)?;
        Ok(sha256)
    }

    #[must_use]
    pub fn scope(&self) -> &SessionScope {
        &self.scope
    }

    pub fn propose(
        &self,
        id: &str,
        source_sha256: &str,
        target: Vec<u8>,
        expected_revision: u64,
    ) -> Result<String, ProviderSessionPolicyOwnerError> {
        self.change_if(|journal| {
            let target_sha256 = crate::sha256_hex(&target);
            if let Some(existing) = journal.proposals.iter().find(|item| item.id == id) {
                if existing.source_sha256 == source_sha256
                    && existing.target_sha256 == target_sha256
                {
                    return Ok((existing.digest.clone(), false));
                }
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            if journal.revision != expected_revision {
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
            Ok((digest, true))
        })
    }

    pub fn approve(
        &self,
        id: &str,
        digest: &str,
        approval_ref: &str,
    ) -> Result<(), ProviderSessionPolicyOwnerError> {
        let revision = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?
            .revision;
        self.approve_at_revision(id, digest, approval_ref, revision)
    }

    pub fn approve_at_revision(
        &self,
        id: &str,
        digest: &str,
        approval_ref: &str,
        expected_revision: u64,
    ) -> Result<(), ProviderSessionPolicyOwnerError> {
        self.change_if(|journal| {
            let proposal = journal
                .proposals
                .iter()
                .find(|item| item.id == id && item.digest == digest)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            if proposal.migration.state == SessionPolicyMigrationState::Approved
                && proposal.migration.approval_ref.as_deref() == Some(approval_ref)
            {
                return Ok(((), false));
            }
            if journal.revision != expected_revision {
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            journal
                .proposals
                .iter_mut()
                .find(|item| item.id == id && item.digest == digest)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?
                .migration
                .approve(approval_ref)
                .map_err(|_| ProviderSessionPolicyOwnerError::Conflict)?;
            Ok(((), true))
        })
    }

    pub fn adopt(
        &self,
        id: &str,
        digest: &str,
        approval_ref: &str,
        expected_revision: u64,
    ) -> Result<(), ProviderSessionPolicyOwnerError> {
        self.change_if(|journal| {
            if let Some(proposal) = journal
                .proposals
                .iter()
                .find(|item| item.id == id && item.digest == digest)
                && proposal.migration.state == SessionPolicyMigrationState::Adopted
                && proposal.migration.approval_ref.as_deref() == Some(approval_ref)
                && journal.active_sha256.as_deref() == Some(proposal.target_sha256.as_str())
            {
                return Ok(((), false));
            }
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
            Ok(((), true))
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

    /// Explicitly activates an already imported, executable policy. This is
    /// separate from migration adoption: a valid initial policy has no
    /// over-limit proposal to approve, but still requires an owner command.
    pub fn adopt_imported(
        &self,
        sha256: &str,
        expected_revision: u64,
    ) -> Result<(), ProviderSessionPolicyOwnerError> {
        self.change_if(|journal| {
            if journal.active_sha256.as_deref() == Some(sha256) {
                return Ok(((), false));
            }
            if journal.revision != expected_revision {
                return Err(ProviderSessionPolicyOwnerError::Conflict);
            }
            let record = journal
                .policies
                .iter()
                .find(|record| record.sha256 == sha256)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            let policy: ProviderSessionPolicy = serde_json::from_slice(&record.bytes)
                .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
            policy
                .admit_for_profile(&self.capabilities)
                .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
            journal.active_sha256 = Some(sha256.to_owned());
            Ok(((), true))
        })
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
    fn change_if<T>(
        &self,
        f: impl FnOnce(&mut Journal) -> Result<(T, bool), ProviderSessionPolicyOwnerError>,
    ) -> Result<T, ProviderSessionPolicyOwnerError> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
        let mut candidate = journal.clone();
        let (result, changed) = f(&mut candidate)?;
        if changed {
            persist_candidate(&self.store, &mut journal, candidate)?;
        }
        Ok(result)
    }
}

fn validate_journal(
    journal: &Journal,
    scope: &SessionScope,
    capabilities: &NativeCapabilities,
) -> Result<(), ProviderSessionPolicyOwnerError> {
    if journal.schema != SCHEMA
        || journal.revision == 0
        || journal.policies.len() > MAX_RECORDS
        || journal.proposals.len() > MAX_RECORDS
    {
        return Err(ProviderSessionPolicyOwnerError::Invalid);
    }
    let mut digests = std::collections::BTreeSet::new();
    for record in &journal.policies {
        if !digests.insert(record.sha256.clone()) {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
        let policy: ProviderSessionPolicy = serde_json::from_slice(&record.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        policy
            .validate_schema()
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        if policy.scope != *scope || crate::sha256_hex(&record.bytes) != record.sha256 {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
    }
    if let Some(active) = &journal.active_sha256 {
        let record = journal
            .policies
            .iter()
            .find(|record| &record.sha256 == active)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let policy: ProviderSessionPolicy = serde_json::from_slice(&record.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        policy
            .admit_for_profile(capabilities)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
    }
    let mut proposal_ids = std::collections::BTreeSet::new();
    for proposal in &journal.proposals {
        let source = journal
            .policies
            .iter()
            .find(|record| record.sha256 == proposal.source_sha256)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let source_policy: ProviderSessionPolicy = serde_json::from_slice(&source.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let target = journal
            .policies
            .iter()
            .find(|record| record.sha256 == proposal.target_sha256)
            .ok_or(ProviderSessionPolicyOwnerError::Invalid)?;
        let target_policy: ProviderSessionPolicy = serde_json::from_slice(&target.bytes)
            .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let canonical_migration = SessionPolicyMigrationProposal::new_from_bytes(
            &source.bytes,
            capabilities,
            &proposal.id,
        )
        .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
        let mut immutable_migration = proposal.migration.clone();
        immutable_migration.state = SessionPolicyMigrationState::Proposed;
        immutable_migration.approval_ref = None;
        immutable_migration.adopted_policy_sha256 = None;
        let expected_digest = serde_json::to_vec(&(
            &proposal.id,
            &proposal.source_sha256,
            &proposal.target_sha256,
            &immutable_migration,
        ))
        .ok()
        .map(crate::sha256_hex);
        if !proposal_ids.insert(proposal.id.clone())
            || !super::valid_id(&proposal.id)
            || expected_digest.as_deref() != Some(proposal.digest.as_str())
            || proposal.migration.validate().is_err()
            || proposal.migration.proposal_id != proposal.id
            || proposal.migration.source_policy_sha256 != proposal.source_sha256
            || proposal.migration.source_policy_id != source_policy.policy_id
            || proposal.migration.source_policy_version != source_policy.version
            || immutable_migration != canonical_migration
            || proposal.migration.target_capabilities_sha256
                != capabilities.binding.descriptor_sha256
            || target_policy.policy_id != source_policy.policy_id
            || target_policy.version <= source_policy.version
            || (proposal.migration.state == SessionPolicyMigrationState::Adopted
                && proposal.migration.adopted_policy_sha256.as_deref()
                    != Some(proposal.target_sha256.as_str()))
            || (proposal.migration.state == SessionPolicyMigrationState::Adopted
                && target_policy.admit_for_profile(capabilities).is_err())
        {
            return Err(ProviderSessionPolicyOwnerError::Invalid);
        }
    }
    Ok(())
}

fn persist_candidate(
    store: &ProviderSessionMetadataStore,
    journal: &mut Journal,
    mut candidate: Journal,
) -> Result<(), ProviderSessionPolicyOwnerError> {
    candidate.revision = candidate
        .revision
        .checked_add(1)
        .ok_or(ProviderSessionPolicyOwnerError::Conflict)?;
    let bytes =
        serde_json::to_vec(&candidate).map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
    store
        .save_owner_journal(&bytes)
        .map_err(|_| ProviderSessionPolicyOwnerError::Store)?;
    *journal = candidate;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn scope() -> SessionScope {
        SessionScope::new("project", "run.policy-owner", "episode", "agent").expect("scope")
    }

    fn path(label: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "sts2-provider-policy-owner-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).expect("directory");
        directory.join("owner.bin")
    }

    fn store(path: &std::path::Path, scope: SessionScope) -> ProviderSessionMetadataStore {
        ProviderSessionMetadataStore::encrypted(path, [11; 32], scope).expect("store")
    }

    fn valid_policy(scope: SessionScope) -> ProviderSessionPolicy {
        let mut policy = ProviderSessionPolicy::disabled(scope);
        policy.mode = super::super::ProviderSessionMode::FixtureOnly;
        policy.credential_realm_ref = "fixture-realm".to_owned();
        policy.profile_sha256 = crate::sha256_hex("fixture-profile");
        policy
    }

    #[test]
    fn exact_import_and_migration_source_bytes_are_retained() {
        let path = path("bytes");
        let scope = scope();
        let owner = ProviderSessionPolicyOwner::open(
            store(&path, scope.clone()),
            scope.clone(),
            NativeCapabilities::fixture(),
        )
        .expect("owner");

        let mut source = valid_policy(scope.clone());
        source.version = 2;
        source.epoch = 2;
        source.max_completed_turns = super::super::MAX_COMPLETED_TURNS + 1;
        let mut source_bytes = serde_json::to_vec_pretty(&source).expect("source");
        source_bytes.push(b'\n');
        let source_sha256 = owner.import(source_bytes.clone()).expect("source import");

        let mut target = source.clone();
        target.version = 3;
        target.epoch = 3;
        target.max_completed_turns = super::super::MAX_COMPLETED_TURNS;
        let target_bytes = serde_json::to_vec(&target).expect("target");
        owner
            .propose(
                "proposal-bytes",
                &source_sha256,
                target_bytes.clone(),
                owner.metadata().expect("metadata").revision,
            )
            .expect("proposal");
        drop(owner);

        let storage = store(&path, scope.clone());
        let journal: Journal =
            serde_json::from_slice(&storage.load_owner_journal().expect("journal"))
                .expect("decode journal");
        let imported = journal
            .policies
            .iter()
            .find(|record| record.sha256 == source_sha256)
            .expect("source record");
        assert_eq!(imported.bytes, source_bytes);
        assert_eq!(
            journal.proposals[0].migration.original_policy_bytes,
            source_bytes
        );
        let target_record = journal
            .policies
            .iter()
            .find(|record| record.sha256 == crate::sha256_hex(&target_bytes))
            .expect("target record");
        assert_eq!(target_record.bytes, target_bytes);
        std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
    }

    #[test]
    fn opening_rejects_policy_digest_and_scope_tampering() {
        for tamper_scope in [false, true] {
            let path = path(if tamper_scope { "scope" } else { "digest" });
            let scope = scope();
            let owner = ProviderSessionPolicyOwner::open(
                store(&path, scope.clone()),
                scope.clone(),
                NativeCapabilities::fixture(),
            )
            .expect("owner");
            owner
                .import(serde_json::to_vec(&valid_policy(scope.clone())).expect("policy"))
                .expect("import");
            drop(owner);

            let storage = store(&path, scope.clone());
            let mut journal: Journal =
                serde_json::from_slice(&storage.load_owner_journal().expect("journal"))
                    .expect("decode journal");
            if tamper_scope {
                let mut policy: ProviderSessionPolicy =
                    serde_json::from_slice(&journal.policies[0].bytes).expect("policy");
                policy.scope.run_id = "foreign-run".to_owned();
                journal.policies[0].bytes = serde_json::to_vec(&policy).expect("encode policy");
                journal.policies[0].sha256 = crate::sha256_hex(&journal.policies[0].bytes);
            } else {
                journal.policies[0].sha256 = "0".repeat(64);
            }
            storage
                .save_owner_journal(&serde_json::to_vec(&journal).expect("encode journal"))
                .expect("save tampered fixture");
            drop(storage);

            let reopened = ProviderSessionPolicyOwner::open(
                store(&path, scope.clone()),
                scope,
                NativeCapabilities::fixture(),
            );
            assert!(reopened.is_err(), "tampered owner state must fail closed");
            std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
        }
    }

    #[test]
    fn opening_rejects_proposal_digest_tampering() {
        let path = path("proposal");
        let scope = scope();
        let owner = ProviderSessionPolicyOwner::open(
            store(&path, scope.clone()),
            scope.clone(),
            NativeCapabilities::fixture(),
        )
        .expect("owner");
        let mut source = valid_policy(scope.clone());
        source.version = 2;
        source.epoch = 2;
        source.max_completed_turns = super::super::MAX_COMPLETED_TURNS + 1;
        let source_sha256 = owner
            .import(serde_json::to_vec(&source).expect("source"))
            .expect("source import");
        let mut target = source.clone();
        target.version = 3;
        target.epoch = 3;
        target.max_completed_turns = super::super::MAX_COMPLETED_TURNS;
        owner
            .propose(
                "proposal-tamper",
                &source_sha256,
                serde_json::to_vec(&target).expect("target"),
                owner.metadata().expect("metadata").revision,
            )
            .expect("proposal");
        drop(owner);

        let storage = store(&path, scope.clone());
        let mut journal: Journal =
            serde_json::from_slice(&storage.load_owner_journal().expect("journal"))
                .expect("decode journal");
        journal.proposals[0].digest = "0".repeat(64);
        storage
            .save_owner_journal(&serde_json::to_vec(&journal).expect("encode journal"))
            .expect("save tampered fixture");
        drop(storage);

        assert!(
            ProviderSessionPolicyOwner::open(
                store(&path, scope.clone()),
                scope,
                NativeCapabilities::fixture(),
            )
            .is_err(),
            "tampered proposal state must fail closed"
        );
        std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
    }
}
