// SPDX-License-Identifier: MIT

// Local lifecycle primitives for projection, backup, cache and retention accounting.
//
// These types deliberately keep source bytes out of their serialized representations.  A
// backup object may carry private bytes in memory for a restore test, but its public metadata is
// digest-addressed and cannot accidentally become a plaintext telemetry or API payload.

pub const MEMORY_BACKUP_SCHEMA: &str = "ascension.context-memory.backup.v1";
pub const MEMORY_PROJECTION_SCHEMA: &str = "ascension.context-memory.projection.v1";
pub const MAX_BACKUP_ENTRIES: usize = MAX_ENTRIES_PER_RUN;
pub const MAX_REVOKED_REFS: usize = MAX_ENTRIES_PER_RUN;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBackupEntry {
    pub metadata: MemoryEntry,
    #[serde(skip)]
    content: Vec<u8>,
    #[serde(skip)]
    protected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBackup {
    pub schema: String,
    pub scope: MemoryScope,
    pub generation: u64,
    pub projection_generation: u64,
    pub revocation_epoch: u64,
    pub revoked: Vec<MemoryRef>,
    pub entries: Vec<MemoryBackupEntry>,
    pub backup_sha256: String,
}

impl MemoryBackup {
    fn validate(&self, scope: &MemoryScope) -> Result<(), MemoryError> {
        if self.schema != MEMORY_BACKUP_SCHEMA
            || self.scope != *scope
            || self.generation > 9_007_199_254_740_991
            || self.projection_generation > self.generation
            || self.revocation_epoch > 9_007_199_254_740_991
            || self.entries.len() > MAX_BACKUP_ENTRIES
            || self.revoked.len() > MAX_REVOKED_REFS
            || self.revoked.iter().any(|reference| !reference.valid())
            || self.revoked.iter().collect::<BTreeSet<_>>().len() != self.revoked.len()
            || !valid_digest(&self.backup_sha256)
        {
            return Err(MemoryError::InvalidEntry);
        }
        let mut references = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for item in &self.entries {
            let tombstone = item.metadata.status == EntryStatus::Revoked;
            if tombstone {
                validate_tombstone(&item.metadata)?;
            } else {
                item.metadata.validate()?;
            }
            if item.metadata.scope != *scope
                || (!tombstone
                    && (item.content.len() != item.metadata.byte_length
                        || sha256_hex(&item.content) != item.metadata.sha256))
                {
                    return Err(MemoryError::InvalidEntry);
                }
            if !references.insert(item.metadata.reference())
                || !identities.insert((item.metadata.entry_id.as_str(), item.metadata.version))
            {
                return Err(MemoryError::Conflict);
            }
        }
        Ok(())
    }

    fn digest_entries(entries: &[MemoryBackupEntry]) -> String {
        let mut bytes = Vec::new();
        for item in entries {
            bytes.extend_from_slice(&serde_json::to_vec(&item.metadata).unwrap_or_default());
            bytes.extend_from_slice(&item.content);
        }
        sha256_hex(bytes)
    }
}

impl MemoryCorpus {
    /// Capture a bounded in-memory restore image.  The serialized form contains only metadata;
    /// callers that need to restore bytes must keep this value inside the private store boundary.
    pub fn backup(&self) -> Result<MemoryBackup, MemoryError> {
        if !self.enabled {
            return Err(MemoryError::Disabled);
        }
        let entries = self
            .entries
            .values()
            .cloned()
            .map(|entry| MemoryBackupEntry {
                content: entry.content.clone(),
                protected: entry.protected,
                metadata: entry,
            })
            .collect::<Vec<_>>();
        let backup_sha256 = MemoryBackup::digest_entries(&entries);
        Ok(MemoryBackup {
            schema: MEMORY_BACKUP_SCHEMA.to_owned(),
            scope: self.scope.clone(),
            generation: self.generation,
            projection_generation: self.projection_generation,
            revocation_epoch: self.revocation_epoch,
            revoked: self.revoked.iter().cloned().collect(),
            entries,
            backup_sha256,
        })
    }

    /// Restore metadata and private bytes atomically.  Revocations already known by this corpus
    /// are unioned with the image before readers are allowed to observe it, so an older backup
    /// cannot resurrect revoked roots, derivatives, snippets or approvals.
    pub fn restore(&mut self, backup: &MemoryBackup) -> Result<(), MemoryError> {
        backup.validate(&self.scope)?;
        if MemoryBackup::digest_entries(&backup.entries) != backup.backup_sha256 {
            return Err(MemoryError::Conflict);
        }
        if !self.enabled {
            self.entries.clear();
            self.total_bytes = 0;
            self.generation = 0;
            self.projection_generation = 0;
            return Ok(());
        }
        let mut candidate = MemoryCorpus::with_limits(
            self.scope.clone(),
            self.max_entries,
            self.max_bytes,
        )?;
        candidate.revocation_epoch = self.revocation_epoch.max(backup.revocation_epoch);
        let backup_revoked = backup.revoked.iter().cloned().collect::<BTreeSet<_>>();
        candidate.revoked = self.revoked.union(&backup_revoked).cloned().collect();
        if candidate.revoked.len() > MAX_REVOKED_REFS {
            return Err(MemoryError::Capacity);
        }
        let mut identities = BTreeSet::new();
        for item in &backup.entries {
            let mut entry = item.metadata.clone();
            entry.content = item.content.clone();
            entry.protected = item.protected;
            if !identities.insert((entry.entry_id.clone(), entry.version)) {
                return Err(MemoryError::Conflict);
            }
            if candidate.entries.insert(entry.reference(), entry).is_some() {
                return Err(MemoryError::Conflict);
            }
        }
        if candidate.entries.len() > candidate.max_entries {
            return Err(MemoryError::Capacity);
        }
        for entry in candidate.entries.values() {
            if entry.status == EntryStatus::Revoked {
                validate_tombstone(entry)?;
                continue;
            }
            entry.validate()?;
            if !valid_lineage_depth(entry, &candidate.entries) {
                return Err(MemoryError::LineageTooDeep);
            }
            for parent in &entry.parents {
                let parent_ref = parent.reference();
                let parent_entry = candidate
                    .entries
                    .get(&parent_ref)
                    .ok_or(MemoryError::MissingParent)?;
                if parent_entry.scope != candidate.scope
                    || parent_entry.observed_seq > entry.observed_seq
                    || parent_entry.admitted_seq > entry.admitted_seq
                    || parent_entry.lineage_depth.saturating_add(1) != entry.lineage_depth
                {
                    return Err(MemoryError::LineageTooDeep);
                }
            }
            candidate.total_bytes = candidate
                .total_bytes
                .saturating_add(entry.content.len());
        }
        if candidate.total_bytes > candidate.max_bytes {
            return Err(MemoryError::Capacity);
        }
        candidate.generation = backup.generation.max(
            candidate
                .entries
                .values()
                .map(|entry| entry.corpus_generation)
                .max()
                .unwrap_or(0),
        );
        candidate.projection_generation = backup.projection_generation.min(candidate.generation);
        candidate.projection_healthy = candidate.projection_generation >= candidate.generation;
        let revoked_refs = candidate
            .entries
            .keys()
            .filter(|reference| candidate.is_revoked(reference, &mut BTreeSet::new()))
            .cloned()
            .collect::<BTreeSet<_>>();
        for reference in revoked_refs {
            if let Some(entry) = candidate.entries.get_mut(&reference) {
                candidate.total_bytes = candidate.total_bytes.saturating_sub(entry.content.len());
                entry.content.clear();
                entry.status = EntryStatus::Revoked;
            }
        }
        *self = candidate;
        Ok(())
    }

    /// Rebuild only the derived reference projection.  It never changes authoritative entries or
    /// their bytes and reports the digest that a reader can pin to a corpus generation.
    pub fn rebuild_projection(&mut self) -> Result<MemoryProjection, MemoryError> {
        if !self.enabled {
            return Err(MemoryError::Disabled);
        }
        let references = self
            .entries
            .values()
            .filter(|entry| {
                entry.status == EntryStatus::Admitted
                    && !self.is_revoked(&entry.reference(), &mut BTreeSet::new())
            })
            .map(MemoryEntry::reference)
            .collect::<Vec<_>>();
        let digest = sha256_hex(serde_json::to_vec(&references).unwrap_or_default());
        self.projection_generation = self.generation;
        self.projection_healthy = true;
        Ok(MemoryProjection {
            schema: MEMORY_PROJECTION_SCHEMA.to_owned(),
            scope: self.scope.clone(),
            corpus_generation: self.generation,
            projection_generation: self.projection_generation,
            reference_count: references.len(),
            projection_sha256: digest,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProjection {
    pub schema: String,
    pub scope: MemoryScope,
    pub corpus_generation: u64,
    pub projection_generation: u64,
    pub reference_count: usize,
    pub projection_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetrievalOutcome {
    Complete,
    NoMatch,
    CandidateLimited,
    TimeLimited,
    ProjectionUnavailable,
    CacheInvalidated,
}

#[must_use]
pub fn retrieval_outcome(response: &RetrievalResponse) -> RetrievalOutcome {
    match response.coverage {
        RetrievalCoverage::CandidateLimited => RetrievalOutcome::CandidateLimited,
        RetrievalCoverage::TimeLimited => RetrievalOutcome::TimeLimited,
        RetrievalCoverage::ProjectionUnavailable => RetrievalOutcome::ProjectionUnavailable,
        RetrievalCoverage::CompleteWithinScope if response.results.is_empty() => {
            RetrievalOutcome::NoMatch
        }
        RetrievalCoverage::CompleteWithinScope => RetrievalOutcome::Complete,
    }
}
