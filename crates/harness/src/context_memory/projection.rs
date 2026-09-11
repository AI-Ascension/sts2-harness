// SPDX-License-Identifier: MIT

pub const MEMORY_PROJECTION_INDEX_SCHEMA: &str = "ascension.context-memory.projection-index.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionFailpoint {
    BeforeSwap,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRecord {
    pub schema: String,
    pub scope: MemoryScope,
    pub corpus_generation: u64,
    pub revocation_epoch: u64,
    pub references: Vec<MemoryRef>,
    pub references_sha256: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectionStore {
    current: Option<ProjectionRecord>,
    outbox: Vec<MemoryRef>,
    failpoint: Option<ProjectionFailpoint>,
}

impl ProjectionStore {
    pub fn set_failpoint(&mut self, failpoint: Option<ProjectionFailpoint>) {
        self.failpoint = failpoint;
    }

    pub fn current(&self) -> Option<&ProjectionRecord> {
        self.current.as_ref()
    }

    pub fn enqueue(&mut self, reference: MemoryRef) -> Result<(), MemoryError> {
        if !reference.valid() || self.outbox.len() >= MAX_ENTRIES_PER_RUN {
            return Err(MemoryError::InvalidEntry);
        }
        self.outbox.push(reference);
        Ok(())
    }

    pub fn corrupt_for_test(&mut self) {
        if let Some(current) = self.current.as_mut() {
            current.references_sha256 = sha256_hex("corrupt-projection");
        }
    }

    pub fn rebuild(&mut self, corpus: &MemoryCorpus) -> Result<ProjectionRecord, MemoryError> {
        if !corpus.enabled() {
            return Err(MemoryError::Disabled);
        }
        let candidate = self.record(corpus, eligible_references(corpus));
        if self.failpoint.take().is_some() {
            return Err(MemoryError::PublicationFailed);
        }
        self.current = Some(candidate.clone());
        self.outbox.clear();
        Ok(candidate)
    }

    pub fn read_refs(&self, corpus: &MemoryCorpus) -> Result<Vec<MemoryRef>, MemoryError> {
        let current = self.current.as_ref().ok_or(MemoryError::ProjectionUnavailable)?;
        if !corpus.enabled()
            || current.schema != MEMORY_PROJECTION_INDEX_SCHEMA
            || current.scope != *corpus.scope()
            || current.corpus_generation != corpus.generation()
            || current.revocation_epoch != corpus.revocation_epoch()
            || digest_references(&current.references) != current.references_sha256
            || current.references.windows(2).any(|pair| pair[0] >= pair[1])
            || current
                .references
                .iter()
                .any(|reference| !reference_is_eligible(corpus, reference))
        {
            return Err(MemoryError::ProjectionUnavailable);
        }
        Ok(current.references.clone())
    }

    pub fn replay_outbox(&mut self, corpus: &MemoryCorpus) -> Result<ProjectionRecord, MemoryError> {
        if !corpus.enabled() {
            return Err(MemoryError::Disabled);
        }
        let mut references = eligible_references(corpus);
        references.extend(
            self.outbox
                .iter()
                .filter(|reference| reference_is_eligible(corpus, reference))
                .cloned(),
        );
        references.sort();
        references.dedup();
        let candidate = self.record(corpus, references);
        if self.failpoint.take().is_some() {
            return Err(MemoryError::PublicationFailed);
        }
        self.current = Some(candidate.clone());
        self.outbox.clear();
        Ok(candidate)
    }

    fn record(&self, corpus: &MemoryCorpus, mut references: Vec<MemoryRef>) -> ProjectionRecord {
        references.sort();
        references.dedup();
        ProjectionRecord {
            schema: MEMORY_PROJECTION_INDEX_SCHEMA.to_owned(),
            scope: corpus.scope().clone(),
            corpus_generation: corpus.generation(),
            revocation_epoch: corpus.revocation_epoch(),
            references_sha256: digest_references(&references),
            references,
        }
    }
}

fn digest_references(references: &[MemoryRef]) -> String {
    sha256_hex(serde_json::to_vec(references).unwrap_or_default())
}

fn eligible_references(corpus: &MemoryCorpus) -> Vec<MemoryRef> {
    corpus
        .entries
        .values()
        .filter(|entry| reference_is_eligible(corpus, &entry.reference()))
        .map(MemoryEntry::reference)
        .collect()
}

fn reference_is_eligible(corpus: &MemoryCorpus, reference: &MemoryRef) -> bool {
    corpus.entries.get(reference).is_some_and(|entry| {
        entry.status == EntryStatus::Admitted
            && !corpus.is_revoked(reference, &mut BTreeSet::new())
    })
}
