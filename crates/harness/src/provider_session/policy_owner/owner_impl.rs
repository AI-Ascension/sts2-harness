// SPDX-License-Identifier: MIT

use super::*;

impl ProviderSessionPolicyOwner {
    pub fn open(
        store: ProviderSessionMetadataStore,
        scope: SessionScope,
        capabilities: NativeCapabilities,
    ) -> Result<Self, ProviderSessionPolicyOwnerError> {
        if !scope.valid()
            || store.mode() != super::super::ProviderSessionMetadataMode::EncryptedPersistent
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
        if policy.scope != self.scope || bytes.len() > super::super::MAX_HISTORY_BYTES {
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
            let target_sha256 = journal
                .proposals
                .iter()
                .find(|item| item.id == id && item.digest == digest)
                .map(|item| item.target_sha256.clone())
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            let target = journal
                .policies
                .iter()
                .find(|item| item.sha256 == target_sha256)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            let target_bytes = target.bytes.clone();
            let proposal = journal
                .proposals
                .iter_mut()
                .find(|item| item.id == id && item.digest == digest)
                .ok_or(ProviderSessionPolicyOwnerError::Missing)?;
            proposal
                .migration
                .adopt_retained_bytes(&target_bytes, &self.capabilities, approval_ref)
                .map_err(|_| ProviderSessionPolicyOwnerError::Invalid)?;
            if proposal.migration.state != SessionPolicyMigrationState::Adopted {
                return Err(ProviderSessionPolicyOwnerError::NotAdopted);
            }
            journal.active_sha256 = Some(target_sha256);
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
        if policy.scope != self.scope || bytes.len() > super::super::MAX_HISTORY_BYTES {
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
}
