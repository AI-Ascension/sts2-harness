// SPDX-License-Identifier: MIT

impl MemoryCorpus {
    pub fn new(scope: MemoryScope) -> Result<Self, MemoryError> {
        Self::with_limits(scope, MAX_ENTRIES_PER_RUN, MAX_CORPUS_BYTES)
    }

    pub fn with_limits(
        scope: MemoryScope,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Self, MemoryError> {
        if !scope.valid()
            || max_entries == 0
            || max_entries > MAX_ENTRIES_PER_RUN
            || max_bytes == 0
            || max_bytes > MAX_CORPUS_BYTES
        {
            return Err(MemoryError::InvalidScope);
        }
        Ok(Self {
            scope,
            entries: BTreeMap::new(),
            max_entries,
            max_bytes,
            total_bytes: 0,
            generation: 0,
            projection_generation: 0,
            projection_healthy: true,
            revocation_epoch: 0,
            revoked: BTreeSet::new(),
            enabled: true,
            failpoint: None,
        })
    }

    pub fn scope(&self) -> &MemoryScope {
        &self.scope
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.entries.clear();
            self.revoked.clear();
            self.total_bytes = 0;
            self.generation = 0;
            self.projection_generation = 0;
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_failpoint(&mut self, failpoint: Option<PublicationFailpoint>) {
        self.failpoint = failpoint;
    }

    pub fn set_projection_health(&mut self, healthy: bool) {
        self.projection_healthy = healthy;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn projection_generation(&self) -> u64 {
        self.projection_generation
    }

    pub fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn entries(&self) -> impl Iterator<Item = &MemoryEntry> {
        self.entries.values()
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn entry(&self, reference: &MemoryRef) -> Option<&MemoryEntry> {
        self.entries.get(reference)
    }

    pub fn read_content(
        &self,
        reference: &MemoryRef,
        branch_id: &str,
        cutoff: u64,
        corpus_generation: u64,
        now: &str,
    ) -> Result<&[u8], MemoryError> {
        let entry = self
            .entries
            .get(reference)
            .ok_or(MemoryError::MissingParent)?;
        self.eligible_entry(entry, branch_id, cutoff, corpus_generation, now, false)?;
        Ok(&entry.content)
    }

    pub fn admit(&mut self, entry: MemoryEntry) -> Result<AdmissionOutcome, MemoryError> {
        if !self.enabled {
            return Err(MemoryError::Disabled);
        }
        entry.validate()?;
        if entry.scope != self.scope {
            return Err(MemoryError::InvalidScope);
        }
        for parent in &entry.parents {
            let parent_ref = parent.reference();
            let parent_entry = self
                .entries
                .get(&parent_ref)
                .ok_or(MemoryError::MissingParent)?;
            if parent_entry.scope != self.scope {
                return Err(MemoryError::CrossScopeParent);
            }
            if parent_entry.observed_seq > entry.observed_seq
                || parent_entry.admitted_seq > entry.admitted_seq
            {
                return Err(MemoryError::FutureParent);
            }
            if parent_entry.lineage_depth.saturating_add(1) != entry.lineage_depth {
                return Err(MemoryError::LineageTooDeep);
            }
        }
        let reference = entry.reference();
        if let Some(existing) = self.entries.values().find(|existing| {
            existing.entry_id == entry.entry_id && existing.version == entry.version
        }) {
            if existing.content == entry.content {
                return Ok(AdmissionOutcome::Duplicate);
            }
            return Err(MemoryError::Conflict);
        }
        if self.entries.len() >= self.max_entries
            || self.total_bytes.saturating_add(entry.content.len()) > self.max_bytes
        {
            return Err(MemoryError::Capacity);
        }
        if self.failpoint.take().is_some() {
            return Err(MemoryError::PublicationFailed);
        }
        self.total_bytes = self.total_bytes.saturating_add(entry.content.len());
        self.generation = self.generation.max(entry.corpus_generation).max(1);
        self.entries.insert(reference, entry);
        self.projection_generation = self.generation;
        Ok(AdmissionOutcome::Inserted)
    }

    fn is_revoked(&self, reference: &MemoryRef, seen: &mut BTreeSet<MemoryRef>) -> bool {
        if !seen.insert(reference.clone()) {
            return true;
        }
        if self.revoked.contains(reference) {
            return true;
        }
        self.entries.get(reference).is_some_and(|entry| {
            entry
                .parents
                .iter()
                .any(|parent| self.is_revoked(&parent.reference(), seen))
        })
    }

    fn eligible_entry(
        &self,
        entry: &MemoryEntry,
        branch_id: &str,
        cutoff: u64,
        corpus_generation: u64,
        now: &str,
        include_protected: bool,
    ) -> Result<(), MemoryError> {
        if entry.scope != self.scope || entry.branch_id != branch_id {
            return Err(MemoryError::PermissionDenied);
        }
        if entry.observed_seq > cutoff {
            return Err(MemoryError::FutureParent);
        }
        if entry.corpus_generation > corpus_generation {
            return Err(MemoryError::FutureParent);
        }
        if !entry.active_at(now) {
            return if entry.status == EntryStatus::Revoked {
                Err(MemoryError::Revoked)
            } else {
                Err(MemoryError::Expired)
            };
        }
        if entry.protected && !include_protected {
            return Err(MemoryError::PermissionDenied);
        }
        if self.is_revoked(&entry.reference(), &mut BTreeSet::new()) {
            return Err(MemoryError::Revoked);
        }
        Ok(())
    }

    pub fn revoke(
        &mut self,
        roots: &[MemoryRef],
        created_at: impl Into<String>,
    ) -> Result<RevocationRecord, MemoryError> {
        if roots.is_empty()
            || roots.len() > 16
            || roots.iter().collect::<BTreeSet<_>>().len() != roots.len()
            || roots.iter().any(|root| !self.entries.contains_key(root))
        {
            return Err(MemoryError::InvalidEntry);
        }
        self.revocation_epoch = self.revocation_epoch.saturating_add(1);
        for root in roots {
            self.revoked.insert(root.clone());
        }
        let affected_derivatives = self
            .entries
            .values()
            .filter(|entry| self.is_revoked(&entry.reference(), &mut BTreeSet::new()))
            .count();
        Ok(RevocationRecord {
            schema: MEMORY_REVOCATION_SCHEMA.to_owned(),
            revocation_id: format!("revoke-{}", self.revocation_epoch),
            scope: self.scope.clone(),
            roots: roots.to_vec(),
            revocation_epoch: self.revocation_epoch,
            denial_committed: true,
            cleanup_status: CleanupStatus::Pending,
            affected_derivatives,
            historical_manifests_rewritten: false,
            created_at: created_at.into(),
        })
    }

    pub fn cleanup_revoked(&mut self) -> usize {
        let mut cleaned = 0;
        let revoked_refs = self
            .entries
            .keys()
            .filter(|reference| self.is_revoked(reference, &mut BTreeSet::new()))
            .cloned()
            .collect::<BTreeSet<_>>();
        for entry in self.entries.values_mut() {
            if revoked_refs.contains(&entry.reference()) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.content.len());
                if !entry.content.is_empty() {
                    entry.content.clear();
                    cleaned += 1;
                }
                entry.status = EntryStatus::Revoked;
            }
        }
        self.projection_generation = self.generation;
        cleaned
    }
}
